use std::collections::HashMap;
use std::error::Error;
use std::ops::Deref;
use std::sync::Arc;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{SystemTime, UNIX_EPOCH},
};

use bip324::{AsyncProtocol, Role};
use bitcoin::consensus::{deserialize, deserialize_partial, serialize, Decodable};
use bitcoin::p2p::message::{NetworkMessage, RawNetworkMessage};
use bitcoin::p2p::{message_network::VersionMessage, Address, ServiceFlags};
use bitcoin::Network;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use traits::Stream;
use types::{ConnectionType, FutureResult, Nonce, SendMessageFailures, V1Header};

pub const USER_AGENT: &str = "rust-bitcoin-test / 0.1.0";
pub const DEFAULT_P2P_PORT: u16 = 18444;
pub const PROTOCOL_VERSION: u32 = 70016;

pub mod errors;
mod traits;
pub mod types;

#[derive(Clone)]
pub struct Connection {
    stream: Arc<dyn Stream>,
}

impl Connection {
    pub async fn outbound(
        socket_addr: impl Into<SocketAddr>,
        connection_type: ConnectionType,
        network: Network,
    ) -> Result<Self, Box<dyn Error>> {
        let stream = TcpStream::connect(socket_addr.into()).await?;
        let stream: Arc<dyn Stream> = match connection_type {
            ConnectionType::V1 => {
                let v1_stream = V1Stream::new(stream, network);
                Arc::new(v1_stream)
            }
            ConnectionType::V2 => {
                let v2_stream = V2Stream::connect_with_handshake(stream, network, Role::Initiator)
                    .await
                    .map_err(|e| Box::new(e))?;
                Arc::new(v2_stream)
            }
        };
        Ok(Self { stream })
    }

    pub async fn outbound_with_version_exchange(
        socket_addr: impl Into<SocketAddr>,
        connection_type: ConnectionType,
        network: Network,
        services_offered: ServiceFlags,
    ) -> Result<Self, Box<dyn Error>> {
        let socket_addr: SocketAddr = socket_addr.into();
        let stream = TcpStream::connect(socket_addr).await?;
        let stream: Arc<dyn Stream> = match connection_type {
            ConnectionType::V1 => {
                let v1_stream = V1Stream::new(stream, network);
                Arc::new(v1_stream)
            }
            ConnectionType::V2 => {
                let v2_stream = V2Stream::connect_with_handshake(stream, network, Role::Initiator)
                    .await
                    .map_err(|e| Box::new(e))?;
                Arc::new(v2_stream)
            }
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_secs();
        let ip = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), socket_addr.port());
        let from_and_recv = Address::new(&ip, services_offered);
        let version = VersionMessage {
            version: PROTOCOL_VERSION,
            services: services_offered,
            timestamp: now as i64,
            receiver: from_and_recv.clone(),
            sender: from_and_recv,
            nonce: 1,
            user_agent: USER_AGENT.to_string(),
            start_height: 0,
            relay: false,
        };
        stream
            .write_message(NetworkMessage::Version(version))
            .await?;

        let version = stream
            .read_message()
            .await?
            .ok_or(Box::new(std::io::Error::other(
                "version message not received.",
            )))?;

        if !matches!(version, NetworkMessage::Version(_)) {
            return Err(Box::new(std::io::Error::other(
                "version message not received as response to version sent",
            )));
        }

        loop {
            let message = stream.read_message().await?;
            if let Some(message) = message {
                if matches!(
                    message,
                    NetworkMessage::SendAddrV2
                        | NetworkMessage::SendCmpct(_)
                        | NetworkMessage::SendHeaders
                ) {
                    continue;
                }
                if matches!(message, NetworkMessage::Verack) {
                    stream.write_message(NetworkMessage::Verack).await?;
                    break;
                }
            }
        }
        Ok(Self { stream })
    }

    pub async fn inbound(
        socket_addr: impl Into<SocketAddr>,
        connection_type: ConnectionType,
        network: Network,
    ) -> Result<Self, Box<dyn Error>> {
        let listen = TcpListener::bind(socket_addr.into()).await?;
        let (stream, _) = listen.accept().await?;
        let stream: Arc<dyn Stream> = match connection_type {
            ConnectionType::V1 => {
                let v1_stream = V1Stream::new(stream, network);
                Arc::new(v1_stream)
            }
            ConnectionType::V2 => {
                let v2_stream = V2Stream::connect_with_handshake(stream, network, Role::Responder)
                    .await
                    .map_err(|e| Box::new(e))?;
                Arc::new(v2_stream)
            }
        };
        Ok(Self { stream })
    }

    pub async fn write_message(&self, message: NetworkMessage) -> Result<(), Box<dyn Error>> {
        self.stream.write_message(message).await
    }

    pub async fn read_message(&self) -> Result<Option<NetworkMessage>, Box<dyn Error>> {
        self.stream.read_message().await
    }
}

pub struct ConnectionPool {
    connections: Arc<Mutex<HashMap<Nonce, Connection>>>,
    count: Arc<Mutex<Nonce>>,
    network: Network,
}

impl ConnectionPool {
    pub fn new(network: Network) -> Self {
        Self {
            connections: Arc::new(HashMap::new().into()),
            count: Arc::new(Nonce(0).into()),
            network,
        }
    }

    pub async fn new_outbound(
        &self,
        socket_addr: impl Into<SocketAddr>,
        connection_type: ConnectionType,
        services_offered: ServiceFlags,
        version_exchange: bool,
    ) -> Result<Nonce, Box<dyn Error>> {
        let conn = if version_exchange {
            Connection::outbound_with_version_exchange(
                socket_addr,
                connection_type,
                self.network,
                services_offered,
            )
            .await?
        } else {
            Connection::outbound(socket_addr, connection_type, self.network).await?
        };
        let mut lock = self.connections.lock().await;
        let mut count_lock = self.count.lock().await;
        count_lock.add();
        lock.insert(*count_lock, conn);
        Ok(*count_lock)
    }

    pub async fn send_all(&self, network_message: NetworkMessage) -> SendMessageFailures {
        let mut fails = Vec::new();
        let lock = self.connections.lock().await;
        for (nonce, conn) in lock.deref() {
            let result = conn.write_message(network_message.clone()).await;
            if result.is_err() {
                fails.push(*nonce);
            }
        }
        SendMessageFailures(fails)
    }

    pub async fn send_from(
        &self,
        nonce: &Nonce,
        network_message: NetworkMessage,
    ) -> Result<(), Box<dyn Error>> {
        let lock = self.connections.lock().await;
        if let Some(conn) = lock.get(nonce) {
            conn.write_message(network_message).await?;
        }
        Ok(())
    }

    pub async fn remove(&self, nonce: &Nonce) {
        let mut lock = self.connections.lock().await;
        lock.remove(nonce);
    }

    pub async fn read_from(&self, nonce: &Nonce) -> Result<Option<NetworkMessage>, Box<dyn Error>> {
        let lock = self.connections.lock().await;
        if let Some(conn) = lock.get(nonce) {
            let msg = conn.read_message().await?;
            return Ok(msg);
        }
        Ok(None)
    }
}

#[derive(Debug)]
struct V1Stream {
    stream: Arc<Mutex<TcpStream>>,
    network: Network,
}

impl V1Stream {
    fn new(stream: TcpStream, network: Network) -> Self {
        Self {
            stream: Arc::new(stream.into()),
            network,
        }
    }

    async fn _read_message(&self) -> Result<Option<NetworkMessage>, Box<dyn Error>> {
        let mut stream = self.stream.lock().await;
        let mut message_buf = vec![0_u8; 24];
        let _ = stream
            .read_exact(&mut message_buf)
            .await
            .map_err(|e| Box::new(e))?;
        let header: V1Header = deserialize_partial(&message_buf)
            .map_err(|e| Box::new(e))?
            .0;
        let mut contents_buf = vec![0_u8; header.length as usize];
        let _ = stream
            .read_exact(&mut contents_buf)
            .await
            .map_err(|e| Box::new(e))?;
        match header.command.as_ref() {
            "block" => {
                let mut buf = contents_buf.as_slice();
                Ok(Some(NetworkMessage::Block(
                    Decodable::consensus_decode(&mut buf).map_err(|e| Box::new(e))?,
                )))
            }
            _ => {
                message_buf.extend_from_slice(&contents_buf);
                let message: RawNetworkMessage =
                    deserialize(&message_buf).map_err(|e| Box::new(e))?;
                Ok(Some(message.payload().clone()))
            }
        }
    }

    async fn _write_message(&self, message: NetworkMessage) -> Result<(), Box<dyn Error>> {
        let mut lock = self.stream.lock().await;
        let raw_net_msg = RawNetworkMessage::new(self.network.magic(), message);
        let bytes = serialize(&raw_net_msg);
        lock.write_all(&bytes).await.map_err(|e| Box::new(e))?;
        lock.flush().await.map_err(|e| Box::new(e))?;
        Ok(())
    }
}

impl Stream for V1Stream {
    fn write_message(&self, message: NetworkMessage) -> FutureResult<(), Box<dyn Error>> {
        Box::pin(self._write_message(message))
    }

    fn read_message(&self) -> FutureResult<Option<NetworkMessage>, Box<dyn Error>> {
        Box::pin(self._read_message())
    }
}

struct V2Stream {
    proto: Arc<Mutex<AsyncProtocol<Compat<OwnedReadHalf>, Compat<OwnedWriteHalf>>>>,
}

impl V2Stream {
    async fn connect_with_handshake(
        stream: TcpStream,
        network: Network,
        role: Role,
    ) -> Result<Self, bip324::ProtocolError> {
        let (reader, writer) = stream.into_split();
        let proto = AsyncProtocol::new(
            network,
            role,
            None,
            None,
            reader.compat(),
            writer.compat_write(),
        )
        .await?;
        Ok(Self {
            proto: Arc::new(proto.into()),
        })
    }

    async fn _read_message(&self) -> Result<Option<NetworkMessage>, Box<dyn Error>> {
        let mut lock = self.proto.lock().await;
        let payload = lock.reader().decrypt().await.map_err(|e| Box::new(e))?;
        let message = bip324::serde::deserialize(payload.contents()).map_err(|e| Box::new(e))?;
        Ok(Some(message))
    }

    async fn _write_message(&self, message: NetworkMessage) -> Result<(), Box<dyn Error>> {
        let mut lock = self.proto.lock().await;
        let encoding = bip324::serde::serialize(message).map_err(|e| Box::new(e))?;
        lock.writer()
            .encrypt(&encoding)
            .await
            .map_err(|e| Box::new(e))?;
        Ok(())
    }
}

impl Stream for V2Stream {
    fn write_message(&self, message: NetworkMessage) -> FutureResult<(), Box<dyn Error>> {
        Box::pin(self._write_message(message))
    }

    fn read_message(&self) -> FutureResult<Option<NetworkMessage>, Box<dyn Error>> {
        Box::pin(self._read_message())
    }
}
