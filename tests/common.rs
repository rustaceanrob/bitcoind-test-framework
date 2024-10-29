use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bitcoin::p2p::{message::NetworkMessage, ServiceFlags};
use bitcoin::Network;
use bitcoind_test_framework::common::{
    types::{ConnectionType, Nonce},
    Connection, ConnectionPool, DEFAULT_P2P_PORT,
};

#[tokio::test]
async fn test_handshake() {
    let ip_addr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let local_host = SocketAddr::new(ip_addr, DEFAULT_P2P_PORT);
    let conn = Connection::outbound_with_version_exchange(
        local_host,
        ConnectionType::V1,
        Network::Regtest,
        ServiceFlags::NONE,
    )
    .await
    .unwrap();

    // SendCmpct
    let _ = conn.read_message().await.unwrap();
    // Ping
    let _ = conn.read_message().await.unwrap();
    // FeeFilter
    let _ = conn.read_message().await.unwrap();

    conn.write_message(NetworkMessage::Ping(420)).await.unwrap();

    if let Some(message) = conn.read_message().await.unwrap() {
        if matches!(message, NetworkMessage::Pong(_)) {
            match message {
                NetworkMessage::Pong(p) => assert_eq!(p, 420),
                _ => (),
            }
        } else {
            panic!("no pong message");
        }
    }
}

#[tokio::test]
async fn test_v2_handshake() {
    let ip_addr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let local_host = SocketAddr::new(ip_addr, DEFAULT_P2P_PORT);
    let conn = Connection::outbound_with_version_exchange(
        local_host,
        ConnectionType::V2,
        Network::Regtest,
        ServiceFlags::NONE,
    )
    .await
    .unwrap();

    // SendCmpct
    let _ = conn.read_message().await.unwrap();
    // Ping
    let _ = conn.read_message().await.unwrap();
    // FeeFilter
    let _ = conn.read_message().await.unwrap();
}

#[tokio::test]
async fn test_conn_pool() {
    let ip_addr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let local_host = SocketAddr::new(ip_addr, DEFAULT_P2P_PORT);
    let pool = ConnectionPool::new(Network::Regtest);
    for _ in 0..=4 {
        pool.new_outbound(local_host, ConnectionType::V1, ServiceFlags::NONE, true)
            .await
            .unwrap();
    }
    let failures = pool.send_all(NetworkMessage::SendHeaders).await;
    assert!(failures.0.is_empty());
    let msg = pool.read_from(&Nonce(3)).await.unwrap();
    assert!(msg.is_some());
}
