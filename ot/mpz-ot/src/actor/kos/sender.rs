use std::collections::HashMap;

use async_trait::async_trait;
use futures::channel::{mpsc, oneshot};
use futures_util::{stream::Fuse, SinkExt, StreamExt};
use mpz_core::{Block, ProtocolMessage};
use mpz_ot_core::kos::msgs::SenderPayload;
use utils_aio::{
    non_blocking_backend::{Backend, NonBlockingBackend},
    sink::IoSink,
    stream::IoStream,
};

use crate::{
    actor::kos::{
        into_kos_sink, into_kos_stream,
        msgs::{ActorMessage, Message, TransferPayload, TransferRequest},
    },
    kos::{Sender, SenderError, SenderKeys},
    CommittedOTReceiver, CommittedOTSenderShared, OTError, OTReceiver, OTSenderShared, OTSetup,
};

use super::SenderActorError;

/// Commands that can be sent to a [`SenderActor`].
enum Command {
    GetKeys(GetKeys),
    SendPayload(SendPayload),
    Shutdown(Shutdown),
}

struct GetKeys {
    id: String,
    /// Used to send back the Result to the caller of the GetKeys command.
    caller_response: oneshot::Sender<Result<SenderKeys, SenderError>>,
}

struct SendPayload {
    id: String,
    payload: SenderPayload,
    /// Used to send back the Result to the caller of the SendPayload command.
    caller_response: oneshot::Sender<Result<(), SenderError>>,
}

struct Shutdown {
    /// Used to send back the Result to the caller of the Shutdown command.
    caller_response: oneshot::Sender<Result<(), SenderActorError>>,
}

#[derive(Default)]
struct State {
    pending_keys: HashMap<String, Result<SenderKeys, SenderError>>,
    pending_callers: HashMap<String, oneshot::Sender<Result<SenderKeys, SenderError>>>,
}

opaque_debug::implement!(State);

/// KOS sender actor.
pub struct SenderActor<BaseOT, Si, St> {
    /// A sink to send messages to the KOS receiver actor.
    sink: Si,
    /// A stream to receive messages from the KOS receiver actor.
    stream: Fuse<St>,

    sender: Sender<BaseOT>,

    state: State,

    /// Used to send commands to this actor.
    command_sender: mpsc::UnboundedSender<Command>,
    /// Used to receive commands to this actor.
    commands: mpsc::UnboundedReceiver<Command>,
}

impl<BaseOT, Si, St> SenderActor<BaseOT, Si, St>
where
    BaseOT: OTSetup + OTReceiver<bool, Block> + ProtocolMessage + Send,
    Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
    St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
{
    /// Creates a new sender actor.
    pub fn new(sender: Sender<BaseOT>, sink: Si, stream: St) -> Self {
        let (buffer_sender, buffer_receiver) = mpsc::unbounded();
        Self {
            sink,
            stream: stream.fuse(),
            sender,
            state: Default::default(),
            command_sender: buffer_sender,
            commands: buffer_receiver,
        }
    }

    /// Sets up the sender with the given number of OTs.
    pub async fn setup(&mut self, count: usize) -> Result<(), SenderActorError> {
        let mut sink = into_kos_sink(&mut self.sink);
        let mut stream = into_kos_stream(&mut self.stream);

        self.sender.setup(&mut sink, &mut stream).await?;
        self.sender.extend(&mut sink, &mut stream, count).await?;

        Ok(())
    }

    /// Sets up the sender with the given number of OTs.
    pub async fn setup_with_delta(
        &mut self,
        delta: Block,
        count: usize,
    ) -> Result<(), SenderActorError> {
        let mut sink = into_kos_sink(&mut self.sink);
        let mut stream = into_kos_stream(&mut self.stream);

        self.sender
            .setup_with_delta(&mut sink, &mut stream, delta)
            .await?;
        self.sender.extend(&mut sink, &mut stream, count).await?;

        Ok(())
    }

    /// Returns a `SharedSender` which implements `Clone`.
    pub fn sender(&self) -> SharedSender {
        SharedSender {
            command_sender: self.command_sender.clone(),
        }
    }

    /// Runs the sender actor.
    pub async fn run(&mut self) -> Result<(), SenderActorError> {
        loop {
            futures::select! {
                // Processes a message received from the Receiver.
                msg = self.stream.select_next_some() => {
                    self.handle_msg(msg?.try_into_actor_message()?)?;
                }
                // Processes a command from a controller.
                cmd = self.commands.select_next_some() => {
                    if let Command::Shutdown(Shutdown { caller_response }) = cmd {
                        _ = caller_response.send(Ok(()));
                        return Ok(());
                    }

                    self.handle_cmd(cmd).await;
                }
            }
        }
    }

    /// Handles commands received from a controller.
    async fn handle_cmd(&mut self, cmd: Command) {
        match cmd {
            Command::GetKeys(GetKeys {
                id,
                caller_response,
            }) => {
                if let Some(keys) = self.state.pending_keys.remove(&id) {
                    _ = caller_response.send(keys);
                } else {
                    // The peer has not requested an OT with this id yet.
                    self.state.pending_callers.insert(id, caller_response);
                }
            }
            Command::SendPayload(SendPayload {
                id,
                payload,
                caller_response,
            }) => {
                let res = self
                    .sink
                    .send(ActorMessage::TransferPayload(TransferPayload { id, payload }).into())
                    .await;

                _ = caller_response.send(res.map_err(SenderError::from));
            }
            Command::Shutdown(_) => unreachable!("shutdown should be handled already"),
        }
    }

    /// Handles a message from the KOS receiver actor.
    fn handle_msg(&mut self, msg: ActorMessage) -> Result<(), SenderActorError> {
        match msg {
            ActorMessage::TransferRequest(TransferRequest { id, derandomize }) => {
                // Reserve the keys for the transfer.
                let keys = self
                    .sender
                    .state_mut()
                    .try_as_extension_mut()
                    .map_err(SenderError::from)
                    .and_then(|sender| {
                        sender
                            .keys(derandomize.count as usize)
                            .map_err(SenderError::from)
                    });

                // Derandomization is cheap, we just do it here.
                let keys = keys
                    .and_then(|mut keys| {
                        keys.derandomize(derandomize)?;
                        Ok(keys)
                    })
                    .map_err(SenderError::from);

                // If there is a pending caller, send the keys to it, otherwise
                // we buffer it.
                if let Some(pending_caller) = self.state.pending_callers.remove(&id) {
                    _ = pending_caller.send(keys);
                } else {
                    self.state.pending_keys.insert(id, keys);
                }
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

impl<BaseOT, Si, St> SenderActor<BaseOT, Si, St>
where
    BaseOT: CommittedOTReceiver<bool, Block> + ProtocolMessage + Send,
    Si: IoSink<Message<BaseOT::Msg>> + Send + Unpin,
    St: IoStream<Message<BaseOT::Msg>> + Send + Unpin,
{
    /// Reveals all messages sent to the receiver.
    ///
    /// # Warning
    ///
    /// Obviously, you should be sure you want to do this before calling this function!
    pub async fn reveal(&mut self) -> Result<(), SenderActorError> {
        self.sink.send(ActorMessage::Reveal.into()).await?;

        self.sender
            .reveal(
                &mut into_kos_sink(&mut self.sink),
                &mut into_kos_stream(&mut self.stream),
            )
            .await
            .map_err(SenderActorError::from)
    }
}

/// KOS Shared Sender controller
#[derive(Clone)]
pub struct SharedSender {
    /// Channel for sending commands to the sender actor.
    command_sender: mpsc::UnboundedSender<Command>,
}

opaque_debug::implement!(SharedSender);

impl SharedSender {
    /// Shuts down the sender actor.
    pub async fn shutdown(&self) -> Result<(), SenderActorError> {
        let (caller_response, receiver) = oneshot::channel();
        self.command_sender
            .unbounded_send(Command::Shutdown(Shutdown { caller_response }))?;

        receiver.await?
    }
}

#[async_trait]
impl OTSenderShared<[Block; 2]> for SharedSender {
    async fn send(&self, id: &str, msgs: &[[Block; 2]]) -> Result<(), OTError> {
        let (caller_response, receiver) = oneshot::channel();
        self.command_sender
            .unbounded_send(Command::GetKeys(GetKeys {
                id: id.to_string(),
                caller_response,
            }))
            .map_err(SenderError::from)?;

        let keys = receiver.await.map_err(SenderError::from)??;
        let msgs = msgs.to_vec();
        let payload = Backend::spawn(move || keys.encrypt_blocks(&msgs)).await?;

        let (caller_response, receiver) = oneshot::channel();
        self.command_sender
            .unbounded_send(Command::SendPayload(SendPayload {
                id: id.to_string(),
                payload,
                caller_response,
            }))
            .map_err(SenderError::from)?;

        receiver
            .await
            .map_err(SenderError::from)?
            .map_err(OTError::from)
    }
}

#[async_trait]
impl<const N: usize> OTSenderShared<[[u8; N]; 2]> for SharedSender {
    async fn send(&self, id: &str, msgs: &[[[u8; N]; 2]]) -> Result<(), OTError> {
        let (caller_response, receiver) = oneshot::channel();
        self.command_sender
            .unbounded_send(Command::GetKeys(GetKeys {
                id: id.to_string(),
                caller_response,
            }))
            .map_err(SenderError::from)?;

        let keys = receiver.await.map_err(SenderError::from)??;
        let msgs = msgs.to_vec();
        let payload = Backend::spawn(move || keys.encrypt_bytes(&msgs)).await?;

        let (caller_response, receiver) = oneshot::channel();
        self.command_sender
            .unbounded_send(Command::SendPayload(SendPayload {
                id: id.to_string(),
                payload,
                caller_response,
            }))
            .map_err(SenderError::from)?;

        receiver
            .await
            .map_err(SenderError::from)?
            .map_err(OTError::from)
    }
}

#[async_trait]
impl<T> CommittedOTSenderShared<T> for SharedSender
where
    SharedSender: OTSenderShared<T>,
{
    async fn reveal(&self) -> Result<(), OTError> {
        // this is no-op, as the reveal is performed using the actor struct after
        // shutdown.
        Ok(())
    }
}
