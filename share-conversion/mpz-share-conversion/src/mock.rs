//! Mocks for testing the share conversion protocol.

use crate::{OTReceiveElement, OTSendElement};

use super::{ConverterReceiver, ConverterSender, ReceiverConfig, SenderConfig};
use mpz_ot::mock::{mock_ot_shared_pair, MockSharedOTReceiver, MockSharedOTSender};
use mpz_share_conversion_core::fields::Field;
use utils_aio::duplex::MpscDuplex;

/// A mock converter sender
pub type MockConverterSender<F> = ConverterSender<F, MockSharedOTSender>;
/// A mock converter receiver
pub type MockConverterReceiver<F> = ConverterReceiver<F, MockSharedOTReceiver>;

/// Creates a mock sender and receiver for testing the share conversion protocol.
#[allow(clippy::type_complexity)]
pub fn mock_converter_pair<F: Field>(
    sender_config: SenderConfig,
    receiver_config: ReceiverConfig,
) -> (
    ConverterSender<F, MockSharedOTSender>,
    ConverterReceiver<F, MockSharedOTReceiver>,
)
where
    MockSharedOTSender: OTSendElement<F>,
    MockSharedOTReceiver: OTReceiveElement<F>,
{
    let (c1, c2) = MpscDuplex::new();

    let (ot_sender, ot_receiver) = mock_ot_shared_pair();

    let sender = ConverterSender::new(sender_config, ot_sender, Box::new(c1));
    let receiver = ConverterReceiver::new(receiver_config, ot_receiver, Box::new(c2));

    (sender, receiver)
}
