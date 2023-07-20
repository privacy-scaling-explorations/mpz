use derive_builder::Builder;

/// KOS15 sender configuration.
#[derive(Debug, Default, Clone, Builder)]
pub struct SenderConfig {
    /// Whether the Sender should commit to the messages.
    #[builder(setter(custom), default = "false")]
    sender_commit: bool,
    /// Whether the Receiver should commit to their choices.
    #[builder(setter(custom), default = "false")]
    receiver_commit: bool,
}

impl SenderConfigBuilder {
    /// Sets the Sender to commit to the messages.
    pub fn sender_commit(&mut self) -> &mut Self {
        self.sender_commit = Some(true);
        self
    }

    /// Sets the Receiver to commit to their choices.
    pub fn receiver_commit(&mut self) -> &mut Self {
        self.receiver_commit = Some(true);
        self
    }
}

impl SenderConfig {
    /// Creates a new builder for SenderConfig.
    pub fn builder() -> SenderConfigBuilder {
        SenderConfigBuilder::default()
    }

    /// Whether the Sender should commit to the messages.
    pub fn sender_commit(&self) -> bool {
        self.sender_commit
    }

    /// Whether the Receiver should commit to their choices.
    pub fn receiver_commit(&self) -> bool {
        self.receiver_commit
    }
}

/// KOS15 receiver configuration.
#[derive(Debug, Default, Clone, Builder)]
pub struct ReceiverConfig {
    /// Whether the Sender should commit to the messages.
    #[builder(setter(custom), default = "false")]
    sender_commit: bool,
    /// Whether the Receiver should commit to their choices.
    #[builder(setter(custom), default = "false")]
    receiver_commit: bool,
}

impl ReceiverConfigBuilder {
    /// Sets the Sender to commit to the messages.
    pub fn sender_commit(&mut self) -> &mut Self {
        self.sender_commit = Some(true);
        self
    }

    /// Sets the Receiver to commit to their choices.
    pub fn receiver_commit(&mut self) -> &mut Self {
        self.receiver_commit = Some(true);
        self
    }
}

impl ReceiverConfig {
    /// Creates a new builder for ReceiverConfig.
    pub fn builder() -> ReceiverConfigBuilder {
        ReceiverConfigBuilder::default()
    }

    /// Whether the Sender should commit to the messages.
    pub fn sender_commit(&self) -> bool {
        self.sender_commit
    }

    /// Whether the Receiver should commit to their choices.
    pub fn receiver_commit(&self) -> bool {
        self.receiver_commit
    }
}
