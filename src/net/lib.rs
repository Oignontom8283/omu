//! Sound over the network.
//!
//! Two blocks of the graph use it: one receives a sound sent by another computer (or another program), one sends the
//! sound of the graph to another computer. The protocol is VBAN (VB-Audio's): audio in small UDP packets, simple to
//! write, simple to read, and understood by Voicemeeter, VB-Audio's own tools and many others, so this program can talk to
//! them with nothing to install on either side.
//!
//! VBAN is meant for a local network: it carries no encryption and no authentication beyond a stream name, which is
//! why a receiving block can be told which computer it accepts sound from.

pub mod opus;

/// True for the error a read gives when a datagram was bigger than the buffer (WSAEMSGSIZE on Windows): the datagram is lost, the
/// socket is fine. Anyone who can reach the port can send such a datagram, so it must never end a stream.
pub(crate) fn datagram_too_big(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(10040)
}
pub mod stream;
pub mod vban;
pub mod web;
