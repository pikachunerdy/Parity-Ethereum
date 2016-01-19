/// Network and general IO module.
///
/// Example usage for craeting a network service and adding an IO handler:
///
/// ```rust
/// extern crate ethcore_util as util;
/// use util::*;
///
/// struct MyHandler;
///
/// struct MyMessage {
/// 	data: u32
/// }
///
/// impl NetworkProtocolHandler<MyMessage> for MyHandler {
///		fn initialize(&mut self, io: &mut NetworkContext<MyMessage>) {
///			io.register_timer(1000);
///		}
///
///		fn read(&mut self, io: &mut NetworkContext<MyMessage>, peer: &PeerId, packet_id: u8, data: &[u8]) {
///			println!("Received {} ({} bytes) from {}", packet_id, data.len(), peer);
///		}
///
///		fn connected(&mut self, io: &mut NetworkContext<MyMessage>, peer: &PeerId) {
///			println!("Connected {}", peer);
///		}
///
///		fn disconnected(&mut self, io: &mut NetworkContext<MyMessage>, peer: &PeerId) {
///			println!("Disconnected {}", peer);
///		}
///
///		fn timeout(&mut self, io: &mut NetworkContext<MyMessage>, timer: TimerToken) {
///			println!("Timeout {}", timer);
///		}
///
///		fn message(&mut self, io: &mut NetworkContext<MyMessage>, message: &MyMessage) {
///			println!("Message {}", message.data);
///		}
/// }
///
/// fn main () {
/// 	let mut service = NetworkService::<MyMessage>::start().expect("Error creating network service");
/// 	service.register_protocol(Box::new(MyHandler), "myproto", &[1u8]);
///
/// 	// Wait for quit condition
/// 	// ...
/// 	// Drop the service
/// }
/// ```
mod host;
mod connection;
mod handshake;
mod session;
mod discovery;
mod service;
mod error;
mod node;

/// TODO [arkpar] Please document me
pub type PeerId = host::PeerId;
/// TODO [arkpar] Please document me
pub type PacketId = host::PacketId;
/// TODO [arkpar] Please document me
pub type NetworkContext<'s,'io,  Message> = host::NetworkContext<'s, 'io, Message>;
/// TODO [arkpar] Please document me
pub type NetworkService<Message> = service::NetworkService<Message>;
/// TODO [arkpar] Please document me
pub type NetworkIoMessage<Message> = host::NetworkIoMessage<Message>;
pub use network::host::NetworkIoMessage::User as UserMessage;
/// TODO [arkpar] Please document me
pub type NetworkError = error::NetworkError;

use io::*;

/// Network IO protocol handler. This needs to be implemented for each new subprotocol.
/// All the handler function are called from within IO event loop.
/// `Message` is the type for message data.
pub trait NetworkProtocolHandler<Message>: Send where Message: Send {
	/// Initialize the handler
	fn initialize(&mut self, _io: &mut NetworkContext<Message>) {}
	/// Called when new network packet received.
	fn read(&mut self, io: &mut NetworkContext<Message>, peer: &PeerId, packet_id: u8, data: &[u8]);
	/// Called when new peer is connected. Only called when peer supports the same protocol.
	fn connected(&mut self, io: &mut NetworkContext<Message>, peer: &PeerId);
	/// Called when a previously connected peer disconnects.
	fn disconnected(&mut self, io: &mut NetworkContext<Message>, peer: &PeerId);
	/// Timer function called after a timeout created with `NetworkContext::timeout`.
	fn timeout(&mut self, _io: &mut NetworkContext<Message>, _timer: TimerToken) {}
	/// Called when a broadcasted message is received. The message can only be sent from a different IO handler.
	fn message(&mut self, _io: &mut NetworkContext<Message>, _message: &Message) {}
}

