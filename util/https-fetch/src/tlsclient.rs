// Copyright 2015, 2016 Ethcore (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

use std::str;
use std::sync::{mpsc, Arc};
use std::io::{self, Read, Cursor, BufReader};

use mio;
use mio::tcp::TcpStream;
use rustls::{self, Session};

use client::{FetchError, ClientLoop, FetchResult};
use url::Url;

#[derive(Debug)]
pub enum TlsClientError {
	Initialization,
	UnexpectedEof,
	Connection(io::Error),
	Writer(io::Error),
	Tls(rustls::TLSError),
}

/// This encapsulates the TCP-level connection, some connection
/// state, and the underlying TLS-level session.
pub struct TlsClient {
	token: mio::Token,
	socket: TcpStream,
	tls_session: rustls::ClientSession,
	writer: Box<io::Write>,
	error: Option<TlsClientError>,
	closing: bool,
	listener: mpsc::Sender<FetchResult>,
}

impl io::Write for TlsClient {
	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		self.tls_session.write(bytes)
	}

	fn flush(&mut self) -> io::Result<()> {
		self.tls_session.flush()
	}
}

impl io::Read for TlsClient {
	fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
		self.tls_session.read(bytes)
	}
}

impl TlsClient {
	pub fn make_config() -> Result<Arc<rustls::ClientConfig>, FetchError> {
		let mut config = rustls::ClientConfig::new();
		// TODO [ToDr] Windows / MacOs support!
		let mut cursor = Cursor::new(if cfg!(feature = "ca-github-only") {
			include_bytes!("./ca-github.crt").to_vec()
		} else {
			include_bytes!("./ca-certificates.crt").to_vec()
		});
		let mut reader = BufReader::new(&mut cursor);
		try!(config.root_store.add_pem_file(&mut reader).map_err(|_| FetchError::ReadingCaCertificates));
		// TODO [ToDr] client certificate?
		Ok(Arc::new(config))
	}

	pub fn new(
		token: mio::Token,
		url: &Url,
		writer: Box<io::Write + Send>,
		sender: mpsc::Sender<FetchResult>,
		) -> Result<Self, FetchError> {
			let res = TlsClient::make_config().and_then(|cfg| {
				TcpStream::connect(url.address()).map(|sock| {
					(cfg, sock)
				}).map_err(Into::into)
			});

			match res {
				Ok((cfg, sock)) => Ok(TlsClient {
					token: token,
					writer: writer,
					socket: sock,
					closing: false,
					error: None,
					tls_session: rustls::ClientSession::new(&cfg, url.hostname()),
					listener: sender,
				}),
				Err(e) => {
					sender.send(Err(e)).unwrap_or_else(|e| warn!("Client initialization error: {:?}", e));
					Err(FetchError::Client(TlsClientError::Initialization))
				}
			}
		}

	/// Called by mio each time events we register() for happen.
	/// Return false if reregistering again.
	pub fn ready(&mut self, event_loop: &mut mio::EventLoop<ClientLoop>, token: mio::Token, events: mio::EventSet) -> bool {
		assert_eq!(token, self.token);

		if events.is_readable() {
			self.do_read();
		}

		if events.is_writable() {
			self.do_write();
		}

		if self.is_closed() {
			trace!("Connection closed");
			let res = self.listener.send(match self.error.take() {
				Some(err) => Err(err.into()),
				None => Ok(()),
			});

			if let Err(e) = res {
				warn!("Finished fetching but listener is not available: {:?}", e);
			}
			return true;
		}

		self.reregister(event_loop);
		false
	}

	pub fn register(&mut self, event_loop: &mut mio::EventLoop<ClientLoop>) {
		event_loop.register(
			&self.socket,
			self.token,
			self.event_set(),
			mio::PollOpt::level() | mio::PollOpt::oneshot()
			).unwrap_or_else(|e| self.error = Some(TlsClientError::Connection(e)));
	}

	fn reregister(&mut self, event_loop: &mut mio::EventLoop<ClientLoop>) {
		event_loop.reregister(
			&self.socket,
			self.token,
			self.event_set(),
			mio::PollOpt::level() | mio::PollOpt::oneshot()
			).unwrap_or_else(|e| self.error = Some(TlsClientError::Connection(e)));
	}

	/// We're ready to do a read.
	fn do_read(&mut self) {
		// Read TLS data.  This fails if the underlying TCP connection is broken.
		let rc = self.tls_session.read_tls(&mut self.socket);
		if let Err(e) = rc {
			trace!("TLS read error: {:?}", e);
			self.closing = true;
			self.error = Some(TlsClientError::Connection(e));
			return;
		}

		// If we're ready but there's no data: EOF.
		if rc.unwrap() == 0 {
			trace!("Unexpected EOF");
			self.error = Some(TlsClientError::UnexpectedEof);
			self.closing = true;
			return;
		}

		// Reading some TLS data might have yielded new TLS messages to process.
		// Errors from this indicate TLS protocol problems and are fatal.
		let processed = self.tls_session.process_new_packets();
		if let Err(e) = processed {
			trace!("TLS error: {:?}", e);
			self.error = Some(TlsClientError::Tls(e));
			self.closing = true;
			return;
		}

		// Having read some TLS data, and processed any new messages, we might have new plaintext as a result.
		// Read it and then write it to stdout.
		let mut plaintext = Vec::new();
		let rc = self.tls_session.read_to_end(&mut plaintext);
		if !plaintext.is_empty() {
			self.writer.write(&plaintext).unwrap_or_else(|e| {
				trace!("Write error: {:?}", e);
				self.error = Some(TlsClientError::Writer(e));
				0
			});
		}

		// If that fails, the peer might have started a clean TLS-level session closure.
		if let Err(err) = rc {
			if err.kind() != io::ErrorKind::ConnectionAborted {
				self.error = Some(TlsClientError::Connection(err));
			}
			self.closing = true;
		}
	}

	fn do_write(&mut self) {
		self.tls_session.write_tls(&mut self.socket).unwrap_or_else(|e| {
			warn!("TLS write error: {:?}", e);
		});
	}

	// Use wants_read/wants_write to register for different mio-level IO readiness events.
	fn event_set(&self) -> mio::EventSet {
		let rd = self.tls_session.wants_read();
		let wr = self.tls_session.wants_write();

		if rd && wr {
			mio::EventSet::readable() | mio::EventSet::writable()
		} else if wr {
			mio::EventSet::writable()
		} else {
			mio::EventSet::readable()
		}
	}

	fn is_closed(&self) -> bool {
		self.closing
	}
}

