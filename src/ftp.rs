#![crate_name = "ftp"]
#![crate_type = "lib"]

//! ftp is an FTP client written in Rust.
//!
//! ### Usage
//!
//! Here is a basic usage:
//!
//! ```rust
//! use ftp::FtpStream;
//! let mut ftp_stream = match FtpStream::connect("127.0.0.1", 21) {
//!   Ok(s) => s,
//!   Err(e) => panic!("{}", e)
//! };
//! let _ = ftp_stream.quit();
//! ```
//!


extern crate regex;

use std::io::{Error, ErrorKind, Read, Result, BufReader, BufWriter , Cursor, Write, copy};
use std::net::TcpStream;
use std::string::String;
use std::str::FromStr;
use regex::Regex;

/// Stream to interface with the FTP server. This interface is only for the command stream.
#[derive(Debug)]
pub struct FtpStream {
	command_stream: TcpStream,
	pub host: String,
	pub command_port: u16
}

impl FtpStream {

	/// Creates an FTP Stream.
	pub fn connect<S: Into<String>>(host: S, port: u16) -> Result<FtpStream> {
        let host_string = host.into();
		let connect_string = format!("{}:{}", host_string, port);
		let tcp_stream = try!(TcpStream::connect(&*connect_string));
		let mut ftp_stream = FtpStream {
			command_stream: tcp_stream,
			host: host_string,
			command_port: port
		};
		
		try!(ftp_stream.read_response(220));
		Ok(ftp_stream)
	}

	fn write_str(&mut self, s: &str) -> Result<()> {
		return self.command_stream.write_fmt(format_args!("{}", s));
	}

	/// Log in to the FTP server.
	pub fn login(&mut self, user: &str, password: &str) -> Result<()> {
		let user_command = format!("USER {}\r\n", user);
		try!(self.write_str(&user_command));
		
		self.read_response(331).and_then(|_| {
			let pass_command = format!("PASS {}\r\n", password);
			try!(self.write_str(&pass_command));
			try!(self.read_response(230));
			Ok(())
		})
	}

	/// Change the current directory to the path specified.
	pub fn change_dir(&mut self, path: &str) -> Result<()> {
		let cwd_command = format!("CWD {}\r\n", path);

		try!(self.write_str(&cwd_command));
		try!(self.read_response(250));
		Ok(())
	}

	/// Move the current directory to the parent directory.
	pub fn change_dir_to_parent(&mut self) -> Result<()> {
		let cdup_command = format!("CDUP\r\n");

		try!(self.write_str(&cdup_command));
		try!(self.read_response(250));
		Ok(())
	}

	/// Gets the current directory
	pub fn current_dir(&mut self) -> Result<String> {
		fn index_of(string: &str, ch: char) -> isize {
			let mut i = -1;
			let mut index = 0;
			for c in string.chars() {
				if c == ch {
					i = index;
					return i
				}
				index+=1;
			}
			return i;
		}

		fn last_index_of(string: &str, ch: char) -> isize {
			let mut i = -1;
			let mut index = 0;
			for c in string.chars() {
				if c == ch {
					i = index;
				}
				index+=1;
			}
			return i;
		}
		
		let pwd_command = format!("PWD\r\n");

		try!(self.write_str(&pwd_command));
		self.read_response(257).and_then(|(_, line)| {
			let begin = index_of(&line, '"');
			let end = last_index_of(&line, '"');

			if begin == -1 || end == -1 {
				let cause = format!("Invalid PWD Response: {}", line);
				return Err(Error::new(ErrorKind::Other, cause))
			}
			let b = begin as usize;
			let e = end as usize;

			return Ok(line[b+1..e].to_string())
		})
	}

	/// This does nothing. This is usually just used to keep the connection open.
	pub fn noop(&mut self) -> Result<()> {
		let noop_command = format!("NOOP\r\n");
		try!(self.write_str(&noop_command));
		try!(self.read_response(200));
		Ok(())
	}

	/// This creates new directories on the server.
	pub fn make_dir(&mut self, pathname: &str) -> Result<()> {
		let mkdir_command = format!("MKD {}\r\n", pathname);
		try!(self.write_str(&mkdir_command));
		try!(self.read_response(257));
		Ok(())
	}

	/// Runs the PASV command.
	pub fn pasv(&mut self) -> Result<isize> {
		let pasv_command = format!("PASV\r\n");
		try!(self.write_str(&pasv_command));

		//PASV response format : 227 Entering Passive Mode (h1,h2,h3,h4,p1,p2).

		let response_regex = match Regex::new(r"(.*)\(\d+,\d+,\d+,\d+,(\d+),(\d+)\)(.*)") {
			Ok(re) => re,
    		Err(_) => panic!("Invaid Regex!!"),
		};

		self.read_response(227).and_then(|(_, line)| {
			let caps = response_regex.captures(&line).unwrap();
			let caps_2 = match caps.at(2) {
				Some(s) => s,
				None => return Err(Error::new(ErrorKind::Other, "Problems parsing reponse"))
			};
			let caps_3 = match caps.at(3) {
				Some(s) => s,
				None => return Err(Error::new(ErrorKind::Other, "Problems parsing reponse"))
			};
			let first_part_port: isize = FromStr::from_str(caps_2).unwrap();
			let second_part_port: isize = FromStr::from_str(caps_3).unwrap();
			Ok((first_part_port*256)+second_part_port)
		})
	}

	/// Quits the current FTP session.
	pub fn quit(&mut self) -> Result<()> {
		let quit_command = format!("QUIT\r\n");
		try!(self.write_str(&quit_command));
		try!(self.read_response(221));
		Ok(())
	}

	/// Retrieves the file name specified from the server. This method is a more complicated way to retrieve a file. The reader returned should be dropped.
	/// Also you will have to read the response to make sure it has the correct value.
	pub fn retr(&mut self, file_name: &str) -> Result<BufReader<TcpStream>> {
		let retr_command = format!("RETR {}\r\n", file_name);
		let port = try!(self.pasv());

		let connect_string = format!("{}:{}", self.host, port);
		let data_stream = BufReader::new(TcpStream::connect(&*connect_string).unwrap());

		try!(self.write_str(&retr_command));
		self.read_response(150).and_then(|_| {
			Ok(data_stream)
		})
	}

	fn simple_retr_(&mut self, file_name: &str) -> Result<Cursor<Vec<u8>>> {
		let mut data_stream = match self.retr(file_name) {
			Ok(s) => s,
			Err(e) => return Err(e)
		};

		let buffer: &mut Vec<u8> = &mut Vec::new();
		loop {
			let mut buf = [0; 256];
			let len = try!(data_stream.read(&mut buf));
        	if len == 0 {
				break;
			}
        	try!(buffer.write(&buf[0..len]));
		}

		drop(data_stream);

		Ok(Cursor::new(buffer.clone()))
	}

	/// Simple way to retr a file from the server. This stores the file in memory.
	pub fn simple_retr(&mut self, file_name: &str) -> Result<Cursor<Vec<u8>>> {
		let r = try!(self.simple_retr_(file_name));
		try!(self.read_response(226));
		Ok(r)
	}

	/// Removes the remote pathname from the server.
	pub fn remove_dir(&mut self, pathname: &str) -> Result<()> {
		let rmd_command = format!("RMD {}\r\n", pathname);
		try!(self.write_str(&rmd_command));
		try!(self.read_response(250));
		Ok(())
	}

	fn stor_<R: Read>(&mut self, filename: &str, r: &mut R) -> Result<()> {
		let stor_command = format!("STOR {}\r\n", filename);
		let port = try!(self.pasv());

		let connect_string = format!("{}:{}", self.host, port);
		let data_stream: &mut BufWriter<TcpStream> = &mut BufWriter::new(TcpStream::connect(&*connect_string).unwrap());

		try!(self.write_str(&stor_command));
		try!(self.read_response(150));

		try!(copy(r, data_stream));
		Ok(())
	}

	/// This stores a file on the server.
	pub fn stor<R: Read>(&mut self, filename: &str, r: &mut R) -> Result<()> {
		try!(self.stor_(filename, r));
		try!(self.read_response(226));
		Ok(())
	}

	//Retrieve single line response
	pub fn read_response(&mut self, expected_code: isize) -> Result<(isize, String)> {
		//Carriage return
		let cr = 0x0d;
		//Line Feed
		let lf = 0x0a;
		let mut line_buffer: Vec<u8> = Vec::new();

		while line_buffer.len() < 2 || (line_buffer[line_buffer.len()-1] != lf && line_buffer[line_buffer.len()-2] != cr) {
				let byte_buffer: &mut [u8] = &mut [0];
				match self.command_stream.read(byte_buffer) {
					Ok(_) => {},
					Err(_) => return Err(Error::new(ErrorKind::Other, "Error reading response")),
				}
				line_buffer.push(byte_buffer[0]);
		}

		let response = String::from_utf8(line_buffer).unwrap();
		let chars_to_trim: &[char] = &['\r', '\n'];
		let trimmed_response = response.trim_matches(chars_to_trim);
    	let trimmed_response_vec: Vec<char> = trimmed_response.chars().collect();
    	if trimmed_response_vec.len() < 5 || trimmed_response_vec[3] != ' ' {
    		return Err(Error::new(ErrorKind::Other, "Invalid response"));
    	}

    	let v: Vec<&str> = trimmed_response.splitn(2, ' ').collect();
    	let code: isize = FromStr::from_str(v[0]).unwrap();
    	let message = v[1];
    	if code != expected_code {
    		return Err(Error::new(ErrorKind::Other, format!("Invalid response: {} {}", code, message)))
    	}
    	Ok((code, message.to_string()))
	}
}
