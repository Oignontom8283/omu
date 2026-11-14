# USB/IP server and USB audio devices

This folder is a modified copy of [`usbip-audio`](https://github.com/network-audio/usbip-audio) by Stephen von Takach,
taken at commit `58032ab`. It is distributed under the MIT terms in [usbip-audio.MIT.txt](usbip-audio.MIT.txt).

It serves emulated USB audio devices over USB/IP; the signed `usbip-win2` driver attaches them and Windows treats them
as ordinary sound devices (this is how the virtual microphones exist).

## What was changed

- the files became a module of this crate (`crate::` paths became `super::`), and `rustfmt` was applied to them;
- the Windows timer calls use the `windows` crate instead of `windows-sys`;
- the server can tell whether a device is currently attached (`Server::is_attached`);
- messages go through `tracing` instead of `eprintln!`;
- the UAC1 speaker, the `usbip.exe` helper and the library entry point were left out; the integration tests became
  `protocol_tests.rs`.

The changes are small and may be offered to the original project later.
