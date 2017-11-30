// Copyright 2016 Google Inc. All rights reserved.
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

//! A base for xi plugins. Will be split out into its own crate once it's a bit more stable.

use std::io;
use std::fmt;
use std::path::PathBuf;

use serde_json::{self, Value};

use xi_rpc;
use xi_rpc::{RpcLoop, RpcCtx, RpcCall, RemoteError, ReadError, dict_get_u64, dict_get_string};

#[derive(Debug)]
pub enum Error {
    RpcError(xi_rpc::Error),
    WrongReturnType,
}

// TODO: make more similar to xi_rpc::Handler
pub trait Handler {
    fn call(&mut self, &PluginRequest, PluginCtx) -> Option<Value>;
    #[allow(unused_variables)]
    fn idle(&mut self, ctx: PluginCtx, token: usize) {}
}

//TODO: share this between core and plugin lib
#[derive(Serialize, Deserialize, Debug)]
pub struct ScopeSpan {
    pub start: usize,
    pub end: usize,
    pub scope_id: u32,
}

impl ScopeSpan {
	pub fn new(start: usize, end: usize, scope_id: u32) -> Self {
		ScopeSpan { start, end, scope_id }
	}
}

pub struct PluginCtx<'a>(&'a RpcCtx);

impl<'a> PluginCtx<'a> {
    pub fn get_data(&self, view_id: &str, offset: usize,
                    max_size: usize, rev: u64) -> Result<String, Error> {
        let params = json!({
            "view_id": view_id,
            "offset": offset,
            "max_size": max_size,
            "rev": rev,
        });
        let result = self.send_rpc_request("get_data", &params);
        match result {
            Ok(Value::String(s)) => Ok(s),
            Ok(_) => Err(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }

    pub fn add_scopes(&self, view_id: &str, scopes: &Vec<Vec<String>>) {
        let params = json!({
            "view_id": view_id,
            "scopes": scopes,
        });
        self.send_rpc_notification("add_scopes", &params);
    }

    pub fn update_spans(&self, view_id: &str, start: usize, len: usize, rev: u64, spans: &[ScopeSpan]) {
        let params = json!({
            "view_id": view_id,
            "start": start,
            "len": len,
            "rev": rev,
            "spans": spans,
        });
        self.send_rpc_notification("update_spans", &params);
    }

    fn send_rpc_notification(&self, method: &str, params: &Value) {
        self.0.get_peer().send_rpc_notification(method, params)
    }

    fn send_rpc_request(&self, method: &str, params: &Value) -> Result<Value, xi_rpc::Error> {
        self.0.get_peer().send_rpc_request(method, params)
    }

    /// Determines whether an incoming request (or notification) is pending. This
    /// is intended to reduce latency for bulk operations done in the background.
    pub fn request_is_pending(&self) -> bool {
        self.0.get_peer().request_is_pending()
    }

    /// Schedule the idle handler to be run when there are no requests pending.
    pub fn schedule_idle(&mut self, token: usize) {
        self.0.schedule_idle(token);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum EditType {
    Insert,
    Delete,
    Undo,
    Redo,
    Other,
}

impl EditType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "insert" => EditType::Insert,
            "delete" => EditType::Delete,
            "undo" => EditType::Undo,
            "redo" => EditType::Redo,
            _ => EditType::Other,
        }
    }
}

pub enum PluginRequest<'a> {
    Ping,
    Initialize(PluginBufferInfo),
    Update {
        start: usize,
        end: usize,
        new_len: usize,
        rev: u64,
        edit_type: EditType,
        author: &'a str,
        text: Option<&'a str>,
    },
    DidSave {
        path: PathBuf,
    }
}

//TODO: this is just copy-paste from core-lib::plugins::rpc_types
//these should be shared, it looks like

/// Buffer information sent on plugin init.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginBufferInfo {
    pub buffer_id: usize,
    pub views: Vec<String>,
    pub rev: u64,
    pub buf_size: usize,
    pub nb_lines: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub syntax: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BufferInfoWrapper {
    pub buffer_info: Vec<PluginBufferInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SaveWrapper {
    pub path: PathBuf,
}

enum InternalError {
    InvalidParams,
    UnknownMethod(String),
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            InternalError::UnknownMethod(ref method) => write!(f, "Unknown method {}", method),
            InternalError::InvalidParams => write!(f, "Invalid params"),
        }
    }
}

fn parse_plugin_request<'a>(method: &str, params: &'a Value) ->
        Result<PluginRequest<'a>, InternalError> {
            use self::PluginRequest::*;
    match method {
        "ping" => Ok(Ping),
        "initialize" => {
            match serde_json::from_value::<BufferInfoWrapper>(params.to_owned()) {
                //TODO: this can return multiple values but we assume only one.
                // global plugins will need to correct this assumption.
                Ok(BufferInfoWrapper { mut buffer_info }) => Ok(Initialize(buffer_info.remove(0))),
                Err(_) => {
                    eprintln!("bad params? {:?}", params);
                    Err(InternalError::InvalidParams)
                }
            }
        }
        "did_save" => {
            match serde_json::from_value::<SaveWrapper>(params.to_owned()) {
                Ok(SaveWrapper { path }) => Ok(DidSave { path }),
                Err(_) => {
                    eprintln!("bad params? {:?}", params);
                    Err(InternalError::InvalidParams)
                }
            }
        }
        "update" => {
            params.as_object().and_then(|dict|
                if let (Some(start), Some(end), Some(new_len), Some(rev), Some(edit_type), Some(author)) =
                    (dict_get_u64(dict, "start"), dict_get_u64(dict, "end"),
                        dict_get_u64(dict, "new_len"), dict_get_u64(dict, "rev"),
                        dict_get_string(dict, "edit_type"), dict_get_string(dict, "author")) {
                        Some(PluginRequest::Update {
                            start: start as usize,
                            end: end as usize,
                            new_len: new_len as usize,
                            rev: rev,
                            edit_type: EditType::from_str(edit_type),
                            author: author,
                            text: dict_get_string(dict, "text"),
                        })
                } else { None }
            ).ok_or_else(|| InternalError::InvalidParams)
        }
        _ => Err(InternalError::UnknownMethod(method.to_string()))
    }
}

struct MyHandler<'a, H: 'a>(&'a mut H);

impl<'a, H: Handler> xi_rpc::Handler for MyHandler<'a, H> {
    type Notification = RpcCall;
    type Request = RpcCall;
    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        match parse_plugin_request(&rpc.method, &rpc.params) {
            Ok(req) => {
                if let Some(_) = self.0.call(&req, PluginCtx(ctx)) {
                    eprintln!("Unexpected return value for notification {}", &rpc.method)
                }
            }
            Err(err) => eprintln!("error: {}", err)
        }
    }

    fn handle_request(&mut self, ctx: &RpcCtx, rpc: Self::Request) ->
        Result<Value, RemoteError> {
        match parse_plugin_request(&rpc.method, &rpc.params) {
            Ok(req) => {
                let result = self.0.call(&req, PluginCtx(ctx));
                Ok(result.expect("return value missing"))
            }
            Err(err) => {
                eprintln!("Error {} decoding RPC request {}", err, &rpc.method);
                Err(RemoteError::InvalidRequest(Some(Value::String(err.to_string()))))
            }
        }
    }

    fn idle(&mut self, ctx: &RpcCtx, token: usize) {
        self.0.idle(PluginCtx(ctx), token);
    }

    fn trace_name(&self) -> &'static str {
        "xi-plugin"
    }
}

pub fn mainloop<H: Handler>(handler: &mut H) -> Result<(), ReadError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);
    let mut my_handler = MyHandler(handler);

    rpc_looper.mainloop(|| stdin.lock(), &mut my_handler)
}
