pub trait FromResponse: Sized {
    fn from_response(response: Response) -> Result<Self, LiteError>;
}

impl FromResponse for MasterchainInfo {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::MasterchainInfo(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for MasterchainInfoExt {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::MasterchainInfoExt(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for CurrentTime {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::CurrentTime(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for Version {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::Version(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for BlockData {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::BlockData(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for BlockState {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::BlockState(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for BlockHeader {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::BlockHeader(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for SendMsgStatus {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::SendMsgStatus(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for AccountState {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::AccountState(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for RunMethodResult {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::RunMethodResult(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for ShardInfo {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::ShardInfo(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for AllShardsInfo {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::AllShardsInfo(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for TransactionInfo {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::TransactionInfo(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for TransactionList {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::TransactionList(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for TransactionId {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::TransactionId(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for BlockTransactions {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::BlockTransactions(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for BlockTransactionsExt {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::BlockTransactionsExt(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for PartialBlockProof {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::PartialBlockProof(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for ConfigInfo {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::ConfigInfo(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for ValidatorStats {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::ValidatorStats(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for LibraryResult {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::LibraryResult(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for LibraryResultWithProof {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::LibraryResultWithProof(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for ShardBlockProof {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::ShardBlockProof(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for LookupBlockResult {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::LookupBlockResult(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for OutMsgQueueSizes {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::OutMsgQueueSizes(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for BlockOutMsgQueueSize {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::BlockOutMsgQueueSize(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for DispatchQueueInfo {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::DispatchQueueInfo(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for DispatchQueueMessages {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::DispatchQueueMessages(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for NonfinalValidatorGroups {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::NonfinalValidatorGroups(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for NonfinalCandidate {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::NonfinalCandidate(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for NonfinalPendingShardBlocks {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::NonfinalPendingShardBlocks(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}

impl FromResponse for Error {
    fn from_response(response: Response) -> Result<Self, LiteError> {
        match response {
            Response::Error(s) => Ok(s),
            _ => Err(LiteError::UnexpectedMessage),
        }
    }
}
use crate::liteclient::types::LiteError;
use tonutils_tl::response::*;
