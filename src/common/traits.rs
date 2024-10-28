use std::error::Error;

use bip324::serde::NetworkMessage;

use super::types::FutureResult;

pub(crate) trait Stream: Send + Sync + Unpin {
    fn write_message(&self, message: NetworkMessage) -> FutureResult<(), Box<dyn Error>>;

    fn read_message(&self) -> FutureResult<Option<NetworkMessage>, Box<dyn Error>>;
}
