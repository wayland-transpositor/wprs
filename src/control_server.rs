// Copyright 2024 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/// A simple out-of-band control server for querying/modifying options at
/// runtime. A program talking to this server should send newline-terminated
/// commands and will receive newline-terminated responses containing
/// JSON-serialized Responses. The requests/responses for the user-provided
/// handler may use any JSON-serializable encoding they wish, including JSON
/// strings.
use std::io::BufRead;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;

use wprs_common::utils;

use crate::prelude::*;

#[derive(Debug, Clone, Eq, PartialEq, serde_derive::Deserialize, serde_derive::Serialize)]
enum Status {
    Ok,
    Err,
}

#[derive(Debug, Clone, Eq, PartialEq, serde_derive::Deserialize, serde_derive::Serialize)]
struct Response {
    status: Status,
    payload: String,
}

impl From<Result<String>> for Response {
    fn from(result: Result<String>) -> Self {
        match result {
            Ok(payload) => Self {
                status: Status::Ok,
                payload,
            },
            Err(payload) => Self {
                status: Status::Err,
                payload: payload.to_string(),
            },
        }
    }
}

fn control_handler<F: Fn(&str) -> Result<String>>(stream: UnixStream, handler: F) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = BufWriter::new(stream);
    let mut input = String::new();
    loop {
        match reader.read_line(&mut input) {
            Ok(0) => {
                debug!("Got EOF on control stream.");
                return Ok(());
            },
            Ok(n) => {
                debug!("Read {} bytes from control stream: {:?}", n, input);
                input.pop(); // remove the \n
                let resp: Response = handler(&input).into();
                writer
                    .write_all(format!("{}\n", serde_json::to_string(&resp).unwrap()).as_bytes())
                    .location(loc!())?;
                writer.flush().location(loc!())?;
            },
            Err(err) => bail!("Error reading from control stream: {}", err),
        };
        input.clear();
    }
}

/// Starts a control server with a handler function.
///
/// The handler function should accept a single command and return a
/// `Result<String>` for the response. The input commands will not contain the
/// terminated newline (it will be automatically stripped) and the returned
/// response does not need to contain the terminated newline (it will be
/// automatically appended).
pub fn start<P, F>(sock_path: P, handler: F) -> Result<()>
where
    P: AsRef<Path>,
    F: Fn(&str) -> Result<String> + Send + Sync + Clone + 'static,
{
    let listener = utils::bind_user_socket(sock_path).location(loc!())?;

    thread::spawn(move || -> Result<()> {
        loop {
            let accept_result = listener.accept();
            let (stream, _) = log_and_continue!(accept_result);
            let handler = handler.clone();
            thread::spawn(move || {
                control_handler(stream, handler).log_and_ignore(loc!());
            });
        }
    });
    Ok(())
}
