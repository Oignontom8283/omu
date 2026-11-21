# Third-party notices

Some code and data that ship inside this program come under licences that ask for their text to travel with it. They are
all compatible with this program's own licence (GPL-3.0), and are used unmodified unless said otherwise.

## RNNoise (noise reduction), through `nnnoiseless`

The noise reduction block uses RNNoise, a small neural network by the Xiph.Org Foundation that tells voice from noise,
through the Rust rewrite `nnnoiseless` by Joe Neeman (https://github.com/jneem/nnnoiseless), which carries the trained
model. Licence: BSD-3-Clause.

```
Copyright (c) 2020, Joe Neeman
Copyright (c) 2017, Mozilla
Copyright (c) 2007-2017, Jean-Marc Valin
Copyright (c) 2005-2017, Xiph.Org Foundation
Copyright (c) 2003-2004, Mark Borgerding

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of the Xiph.Org Foundation nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE FOUNDATION
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## usbip-audio (virtual microphones and speakers)

The emulated USB audio devices are built on a modified copy of `usbip-audio` (source in `src/virtual_devices/usb/`).
Licence: MIT.

```
MIT License

Copyright (c) 2026 Stephen von Takach

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Symphonia (reading sound files for the soundboards)

Sound files imported into a soundboard (MP3, FLAC, OGG, WAV, AAC, and the sound of MP4, MOV, MKV and WebM videos) are read with `symphonia` and its format and codec crates
(https://github.com/pdeljanov/Symphonia), by Philip Deljanov and contributors. They are used unmodified, and their
source is available at that address. Licence: MPL-2.0 (Mozilla Public License 2.0), whose full text is at
https://www.mozilla.org/MPL/2.0/ and in each of those crates.


## flacenc and claxon (compressed sounds)

Sounds a soundboard stores compressed use the lossless FLAC format, written with `flacenc` by Yotaro Kubo and
contributors (https://github.com/yotarok/flacenc-rs) and read back with `claxon` by Ruud van Asseldonk
(https://github.com/ruuda/claxon). Both are used unmodified. Licence: Apache-2.0, whose full text is at
https://www.apache.org/licenses/LICENSE-2.0 and in each of those crates.

## DeepFilterNet and tract (deep noise reduction)

The "Deep noise reduction" block runs the DeepFilterNet model with the library `deep_filter` by Hendrik Schröter
(https://github.com/Rikorose/DeepFilterNet) on top of `tract` by Sonos (https://github.com/sonos/tract). Licence of both:
MIT or Apache-2.0, whose texts are at https://opensource.org/license/mit and https://www.apache.org/licenses/LICENSE-2.0
and in each project. The trained model is not part of this program: it is downloaded once, when the user switches the
DeepFilterNet module on, from the project's repository, and checked against a SHA-256 written in the program.

## subst (text templates)

The blocks that assemble text use the `subst` library (https://github.com/fizyr/subst) to read templates such as
`Hello $arg1` and to write the values in them. Licence: BSD-2-Clause (it is also offered under Apache-2.0).

```
Copyright 2022, Fizyr B.V. <info@fizyr.com>
Copyright 2022, Maarten de Vries <maarten@de-vri.es>
Copyright 2022, The Contributors

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## CodeMirror (the editor of text templates)

The field of a text template is a CodeMirror editor (https://codemirror.net, by Marijn Haverbeke and contributors), through the
packages `@codemirror/state`, `@codemirror/view` and `@codemirror/autocomplete`. Licence: MIT.

```
MIT License

Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## Certificates (secure connections)

The choice of the certificates the program trusts, and the secure connections it makes, use these libraries, each offered under the
MIT licence or the Apache-2.0 licence (at your choice); their full licence texts are in their source packages:

- `rustls` (https://github.com/rustls/rustls) and `rustls-pki-types`, the TLS implementation and its certificate types;
- `rustls-native-certs` (https://github.com/rustls/rustls-native-certs), to read the certificates the operating system trusts
  (Apache-2.0, ISC or MIT);
- `rustls-platform-verifier`, through `ureq` (https://github.com/algesten/ureq), the HTTP client;
- `x509-parser` (https://github.com/rusticata/x509-parser), to read who a certificate is about and until when it is valid;
- `rcgen` (https://github.com/rustls/rcgen), to make the certificate the daemon shows over HTTPS, and the ones the tests use;
- `axum-server` (https://github.com/programatik29/axum-server), the secure (HTTPS) server: MIT.
- `pem` (https://github.com/jcreekmore/pem-rs), to write certificates as PEM text for FFmpeg: MIT.

## WebRTC and Opus (sound through the web)

- `str0m` (https://github.com/algesten/str0m), the WebRTC implementation (ICE, DTLS, SRTP), with the cryptography of the RustCrypto
  project: MIT or Apache-2.0, at your choice.
- `unsafe-libopus` (https://github.com/DCNick3/unsafe-libopus), the Opus codec, which is `libopus` of the Xiph.Org Foundation
  translated to Rust. Licence of libopus (BSD-3-Clause):

```
Copyright 2001-2011 Xiph.Org, Skype Limited, Octasic, Jean-Marc Valin, Timothy B. Terriberry, CSIRO, Gregory Maxwell,
Mark Borgerding, Erik de Castro Lopo. Redistribution and use in source and binary forms, with or without modification, are
permitted provided that the following conditions are met: redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer; redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided
with the distribution; neither the name of Internet Society, IETF or IETF Trust, nor the names of specific contributors, may
be used to endorse or promote products derived from this software without specific prior written permission. THIS SOFTWARE
IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED.
```

- `if-addrs` (https://github.com/messense/if-addrs), to list the addresses of the computer's networks: MIT or BSD-3-Clause.
