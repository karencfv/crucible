// Copyright 2022 Oxide Computer Company
use super::*;
use tokio::sync::oneshot;

/**
 * Couple a BlockOp with a notifier for calling code. This uses a single-use
 * channel to send the result of a particular operation, and is meant to be
 * paired with a BlockReqWaiter.
 */
#[must_use]
#[derive(Debug)]
pub(crate) struct BlockReq {
    pub op: BlockOp,
    pub res: BlockRes,
}

#[must_use]
#[derive(Debug)]
pub(crate) struct BlockRes(Option<oneshot::Sender<Result<(), CrucibleError>>>);
impl BlockRes {
    /// Consume this BlockRes and send Ok to the receiver
    pub fn send_ok(self) {
        self.send_result(Ok(()))
    }

    /// Consume this BlockRes and send an Err to the receiver
    pub fn send_err(self, e: CrucibleError) {
        self.send_result(Err(e))
    }

    /// Consume this BlockRes and send a Result to the receiver
    fn send_result(mut self, r: Result<(), CrucibleError>) {
        // XXX this eats the result!
        let _ = self.0.take().expect("sender was populated").send(r);
    }
}
impl Drop for BlockRes {
    fn drop(&mut self) {
        if self.0.is_some() {
            // During normal operation, we expect to reply to every BlockReq, so
            // we'll fire a DTrace probe here.
            cdt::up__block__req__dropped!();
        }
    }
}

/**
 * When BlockOps are sent to a guest, the calling function receives a waiter
 * that it can block on. This uses a single-use channel to receive the
 * result of a particular operation, and is meant to be paired with a
 * BlockReq.
 */
#[must_use]
pub(crate) struct BlockReqWaiter {
    recv: oneshot::Receiver<Result<(), CrucibleError>>,
}

impl BlockReqWaiter {
    /// Create associated `BlockReqWaiter`/`BlockRes` pair
    pub fn pair() -> (BlockReqWaiter, BlockRes) {
        let (send, recv) = oneshot::channel();
        (Self { recv }, BlockRes(Some(send)))
    }

    /// Consume this BlockReqWaiter and wait on the message
    ///
    /// If the other side of the oneshot drops without a reply, log an error
    pub async fn wait(self, log: &Logger) -> Result<(), CrucibleError> {
        match self.recv.await {
            Ok(v) => v,
            Err(_) => {
                warn!(
                    log,
                    "BlockReqWaiter disconnected; \
                     this should only happen at exit"
                );
                Err(CrucibleError::RecvDisconnected)
            }
        }
    }

    #[cfg(test)]
    pub fn try_wait(&mut self) -> Option<Result<(), CrucibleError>> {
        match self.recv.try_recv() {
            Ok(v) => Some(v),
            Err(e) => match e {
                oneshot::error::TryRecvError::Empty => None,
                oneshot::error::TryRecvError::Closed => {
                    Some(Err(CrucibleError::RecvDisconnected))
                }
            },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_blockreq_and_blockreqwaiter() {
        let (brw, res) = BlockReqWaiter::pair();

        res.send_ok();

        brw.wait(&crucible_common::build_logger()).await.unwrap();
    }

    #[tokio::test]
    async fn test_blockreq_and_blockreqwaiter_err() {
        let (brw, res) = BlockReqWaiter::pair();

        res.send_err(CrucibleError::UpstairsInactive);

        assert!(brw.wait(&crucible_common::build_logger()).await.is_err());
    }
}
