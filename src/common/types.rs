use std::future::Future;
use std::pin::Pin;

use bitcoin::consensus::Decodable;
use bitcoin::io::BufRead;
use bitcoin::p2p::message::CommandString;
use bitcoin::p2p::Magic;

pub(crate) type FutureResult<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

#[derive(Debug, Clone, Copy)]
pub enum ConnectionType {
    V1,
    V2,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd, Hash, Eq, Ord)]
pub struct Nonce(pub u32);

impl Nonce {
    pub(crate) fn add(&mut self) {
        self.0 = self.0 + 1;
    }
}

impl std::ops::Add for Nonce {
    type Output = Nonce;

    fn add(self, rhs: Self) -> Self::Output {
        Nonce(self.0 + rhs.0)
    }
}

pub struct SendMessageFailures(pub Vec<Nonce>);

#[allow(unused)]
pub(crate) struct V1Header {
    pub(crate) magic: Magic,
    pub(crate) command: CommandString,
    pub(crate) length: u32,
    _checksum: u32,
}

impl Decodable for V1Header {
    fn consensus_decode<R: BufRead + ?Sized>(
        reader: &mut R,
    ) -> Result<Self, bitcoin::consensus::encode::Error> {
        let magic = Magic::consensus_decode(reader)?;
        let command = CommandString::consensus_decode(reader)?;
        let length = u32::consensus_decode(reader)?;
        let _checksum = u32::consensus_decode(reader)?;
        Ok(Self {
            magic,
            command,
            length,
            _checksum,
        })
    }
}
