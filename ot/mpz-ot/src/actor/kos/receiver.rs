use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use futures::{
    channel::{mpsc, oneshot},
    stream::Fuse,
    SinkExt, StreamExt,
};
use utils_aio::{
    non_blocking_backend::{Backend, NonBlockingBackend},
    sink::IoSink,
    stream::IoStream,
};

use crate::{
    kos::{Receiver, ReceiverError, ReceiverKeys},
    OTError, OTReceiverShared, OTSetup, VerifiableOTReceiverShared, VerifiableOTSender,
};
use mpz_core::{Block, ProtocolMessage};
use mpz_ot_core::kos::{msgs::SenderPayload, PayloadRecord};

use crate::actor::kos::{
    into_kos_sink, into_kos_stream,
    msgs::{ActorMessage, Message, TransferPayload, TransferRequest},
    ReceiverActorError,
};

enum Command {
    Receive {
        id: String,
        choices: Vec<bool>,
        caller_response: oneshot::Sender<Result<(ReceiverKeys, SenderPayload), ReceiverError>>,
    },
    Verify(Verify),
    Shutdown(Shutdown),
}

struct Verify {
    id: String,
    caller_response: oneshot::Sender<Result<PayloadRecord, ReceiverError>>,
}

struct PendingTransfer {
    keys: ReceiverKeys,
    caller_response: oneshot::Sender<Result<(ReceiverKeys, SenderPayload), ReceiverError>>,
}

opaque_debug::implement!(PendingTransfer);

struct Shutdown {
    caller_response: oneshot::Sender<Result<(), ReceiverActorError>>,
}

#[derive(Default)]
struct State {
    ids: HashMap<String, u32>,

    pending_transfers: HashMap<String, PendingTransfer>,
    pending_verify: VecDeque<Verify>,
}

/// KOS receiver actor.
pub struct ReceiverActor<BaseOT, Si, St> {
    sink: Si,
    stream: Fuse<St>,

    receiver: Receiver<BaseOT>,

    state: State,

    command_sender: mpsc::UnboundedSender<Command>,
    commands: mpsc::UnboundedReceiver<Command>,
}

impl<BaseOT, Si, St> ReceiverActor<BaseOT, Si, St>
where
    // TODO: Support non-verifiable base OT.
    BaseOT: OTSetup + VerifiableOTSender<bool, [Block; 2]> + ProtocolMessage + Send,
    Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
    St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
{
    /// Create a new receiver actor.
    pub fn new(receiver: Receiver<BaseOT>, sink: Si, stream: St) -> Self {
        let (command_sender, commands) = mpsc::unbounded();

        Self {
            receiver,
            sink,
            stream: stream.fuse(),
            state: Default::default(),
            command_sender,
            commands,
        }
    }

    /// Sets up the receiver with the given number of OTs.
    pub async fn setup(&mut self, count: usize) -> Result<(), ReceiverActorError> {
        let mut sink = into_kos_sink(&mut self.sink);
        let mut stream = into_kos_stream(&mut self.stream);

        self.receiver.setup(&mut sink, &mut stream).await?;
        self.receiver.extend(&mut sink, &mut stream, count).await?;

        Ok(())
    }

    /// Returns a `SharedReceiver` which implements `Clone`.
    pub fn receiver(&self) -> SharedReceiver {
        SharedReceiver {
            sender: self.command_sender.clone(),
        }
    }

    /// Run the receiver actor.
    pub async fn run(&mut self) -> Result<(), ReceiverActorError> {
        loop {
            futures::select! {
                msg = self.stream.select_next_some() => self.handle_msg(msg?).await?,
                cmd = self.commands.select_next_some() => {
                    if let Command::Shutdown(Shutdown { caller_response }) = cmd {
                        _ = caller_response.send(Ok(()));
                        return Ok(());
                    }

                    self.handle_cmd(cmd).await?
                },
            }
        }
    }

    async fn start_transfer(
        &mut self,
        id: &str,
        choices: &[bool],
    ) -> Result<ReceiverKeys, ReceiverError> {
        let mut keys = self
            .receiver
            .state_mut()
            .as_extension_mut()?
            .keys(choices.len())?;

        let derandomize = keys.derandomize(choices)?;

        self.sink
            .send(
                ActorMessage::TransferRequest(TransferRequest {
                    id: id.to_string(),
                    derandomize,
                })
                .into(),
            )
            .await?;

        Ok(keys)
    }

    async fn start_verification(&mut self) -> Result<(), ReceiverError> {
        self.receiver
            .verify_delta(
                &mut into_kos_sink(&mut self.sink),
                &mut into_kos_stream(&mut self.stream),
            )
            .await?;

        // Process backlog of pending verifications.
        let backlog = std::mem::take(&mut self.state.pending_verify);
        for verify in backlog {
            self.handle_verify(verify)
        }

        Ok(())
    }

    fn handle_verify(&mut self, verify: Verify) {
        // If we're ready to start verifying we do so, otherwise, we buffer
        // the verification for later.
        if self.receiver.state().is_verify() {
            let Verify {
                id,
                caller_response,
            } = verify;

            if let Some(id) = self.state.ids.get(&id) {
                // Send payload record to the caller.
                _ = caller_response.send(
                    self.receiver
                        .state_mut()
                        .as_verify_mut()
                        .map_err(ReceiverError::from)
                        .and_then(|receiver| {
                            receiver.remove_record(*id).map_err(ReceiverError::from)
                        }),
                );
            } else {
                _ = caller_response.send(Err(ReceiverError::Other(format!(
                    "transfer id not found: {id}"
                ))));
            }
        } else {
            self.state.pending_verify.push_back(verify)
        }
    }

    async fn handle_cmd(&mut self, cmd: Command) -> Result<(), ReceiverError> {
        match cmd {
            Command::Receive {
                id,
                choices,
                caller_response,
            } => {
                let keys = match self.start_transfer(&id, &choices).await {
                    Ok(keys) => keys,
                    Err(e) => {
                        _ = caller_response.send(Err(e));
                        return Ok(());
                    }
                };

                self.state.ids.insert(id.clone(), keys.id());
                self.state.pending_transfers.insert(
                    id,
                    PendingTransfer {
                        keys,
                        caller_response,
                    },
                );
            }
            Command::Verify(verify) => self.handle_verify(verify),
            Command::Shutdown(_) => unreachable!("shutdown should be handled already"),
        }

        Ok(())
    }

    async fn handle_msg(&mut self, msg: Message<BaseOT::Msg>) -> Result<(), ReceiverActorError> {
        let msg = msg.into_actor_message()?;

        match msg {
            ActorMessage::TransferPayload(TransferPayload { id, payload }) => {
                let PendingTransfer {
                    keys,
                    caller_response,
                } = self
                    .state
                    .pending_transfers
                    .remove(&id)
                    .ok_or_else(|| ReceiverActorError::UnexpectedTransferId(id))?;

                _ = caller_response.send(Ok((keys, payload)));
            }
            ActorMessage::Reveal => {
                self.start_verification().await?;
            }
            msg => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unexpected msg: {:?}", msg),
                ))?
            }
        }

        Ok(())
    }
}

/// KOS receiver actor control.
#[derive(Debug, Clone)]
pub struct SharedReceiver {
    sender: mpsc::UnboundedSender<Command>,
}

impl SharedReceiver {
    /// Shuts down the receiver actor.
    pub async fn shutdown(&self) -> Result<(), ReceiverActorError> {
        let (sender, receiver) = oneshot::channel();

        self.sender.unbounded_send(Command::Shutdown(Shutdown {
            caller_response: sender,
        }))?;

        receiver.await?
    }
}

#[async_trait]
impl OTReceiverShared<bool, Block> for SharedReceiver {
    async fn receive(&self, id: &str, choices: &[bool]) -> Result<Vec<Block>, OTError> {
        let (sender, receiver) = oneshot::channel();

        self.sender
            .unbounded_send(Command::Receive {
                id: id.to_string(),
                choices: choices.to_vec(),
                caller_response: sender,
            })
            .map_err(ReceiverError::from)?;

        let (keys, payload) = receiver.await.map_err(ReceiverError::from)??;

        Backend::spawn(move || keys.decrypt_blocks(payload))
            .await
            .map_err(OTError::from)
    }
}

#[async_trait]
impl<const N: usize> OTReceiverShared<bool, [u8; N]> for SharedReceiver {
    async fn receive(&self, id: &str, choices: &[bool]) -> Result<Vec<[u8; N]>, OTError> {
        let (sender, receiver) = oneshot::channel();

        self.sender
            .unbounded_send(Command::Receive {
                id: id.to_string(),
                choices: choices.to_vec(),
                caller_response: sender,
            })
            .map_err(ReceiverError::from)?;

        let (keys, payload) = receiver.await.map_err(ReceiverError::from)??;

        Backend::spawn(move || keys.decrypt_bytes(payload))
            .await
            .map_err(OTError::from)
    }
}

#[async_trait]
impl VerifiableOTReceiverShared<bool, Block, [Block; 2]> for SharedReceiver {
    async fn verify(&self, id: &str, msgs: &[[Block; 2]]) -> Result<(), OTError> {
        let (sender, receiver) = oneshot::channel();

        self.sender
            .unbounded_send(Command::Verify(Verify {
                id: id.to_string(),
                caller_response: sender,
            }))
            .map_err(ReceiverError::from)?;

        let record = receiver.await.map_err(ReceiverError::from)??;

        let msgs = msgs.to_vec();
        Backend::spawn(move || record.verify(&msgs))
            .await
            .map_err(OTError::from)
    }
}
