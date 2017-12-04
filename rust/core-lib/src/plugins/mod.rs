// Copyright 2017 Google Inc. All rights reserved.
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

//! Plugins and related functionality.

pub mod rpc_types;
mod manager;
mod manifest;
mod catalog;

use std::sync::{Arc, Mutex, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::process::{Child, Command as ProcCommand, Stdio};
use std::io::{self, BufReader};

use serde_json::{self, Value};

use xi_rpc::{self, RpcPeer, RpcCtx, RpcLoop, Handler, RemoteError, Trace, Timestamp, CowStr};
use tabs::ViewIdentifier;

pub use self::manager::{PluginManagerRef, WeakPluginManagerRef};
pub use self::manifest::{PluginDescription, Command, PlaceholderRpc};

use self::rpc_types::{PluginUpdate, PluginNotification, PluginRequest,
PluginBufferInfo};

use self::manager::PluginName;
use self::catalog::PluginCatalog;


pub type PluginPeer = RpcPeer;
/// A process-unique identifier for a running plugin.
///
/// Note: two instances of the same executable will have different identifiers.
/// Note: this identifier is distinct from the OS's process id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PluginPid(usize);

/// A running plugin.
pub struct Plugin {
    peer: PluginPeer,
    /// The plugin's process
    process: Child,
    manager: WeakPluginManagerRef,
    description: PluginDescription,
    identifier: PluginPid,
}

/// A convenience wrapper for passing around a reference to a plugin.
///
/// Note: A plugin is always owned by and used through a `PluginRef`.
///
/// The second field is used to flag dead plugins for cleanup.
pub struct PluginRef(Arc<Mutex<Plugin>>, Arc<AtomicBool>);

impl Clone for PluginRef {
    fn clone(&self) -> Self {
        PluginRef(self.0.clone(), self.1.clone())
    }
}

impl Handler for PluginRef {
    type Notification = PluginNotification;
    type Request = PluginRequest;
    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        let plugin_manager = {
            self.0.lock().unwrap().manager.upgrade()
        };
        if let Some(plugin_manager) = plugin_manager {
            let pid = self.get_identifier();
            plugin_manager.lock().handle_plugin_notification(rpc, pid, ctx.get_active_trace())
        }
    }

    fn handle_request(&mut self, ctx: &RpcCtx, rpc: Self::Request) ->
        Result<Value, RemoteError> {
        let plugin_manager = {
            self.0.lock().unwrap().manager.upgrade()
        };
        if let Some(plugin_manager) = plugin_manager {
            let pid = self.get_identifier();
            Ok(plugin_manager.lock().handle_plugin_request(rpc, pid, ctx.get_active_trace()))
        } else {
            Err(RemoteError::custom(88, "Plugin manager missing", None))
        }
    }
}


impl PluginRef {
    /// Send an arbitrary RPC notification to the plugin.
    pub fn rpc_notification(&self, method: &str, params: &Value) {
        self.0.lock().unwrap().peer.send_rpc_notification(method, params);
    }

    /// Initialize the plugin.
    pub fn initialize(&self, init: &[PluginBufferInfo], trace: Timestamp) {
        self.0.lock().unwrap().peer
            .send_trace_rpc_notification("initialize", &json!({
                "buffer_info": init,
            }), trace);
    }

    /// Update message sent to the plugin.
    pub fn update<F>(&self, update: &PluginUpdate, trace: Timestamp, callback: F)
            where F: FnOnce(Result<Value, xi_rpc::Error>) + Send + 'static {
        let params = serde_json::to_value(update).expect("PluginUpdate invalid");
        match self.0.lock() {
            Ok(plugin) =>
                plugin.peer.send_trace_rpc_request_async("update", &params,
                                                         Box::new(callback),
                                                         trace),
            Err(err) => {
                eprintln!("plugin update failed {:?}", err);
                callback(Err(xi_rpc::Error::PeerDisconnect));
            }
        }
    }

    pub fn collect_traces(&self) -> Vec<Trace> {
        let plug_name = self.get_name();
        let host_name: CowStr = format!("xi-host.{}", &plug_name).into();
        let mut traces = self.0.lock().unwrap().peer.collect_traces();
        traces.iter_mut().for_each(|t| t.proc_name = host_name.clone().into());
        let remote_traces = self.0.lock().unwrap().peer.send_rpc_request(
            "xi-rpc.collect_traces",
            &json!({}))
            .unwrap();

        let mut remote_traces: Vec<Trace> = serde_json::from_value(remote_traces).unwrap();

        remote_traces.iter_mut().for_each(|t| t.proc_name = plug_name.clone().into());
        traces.append(&mut remote_traces);
        traces
    }

    /// Termination message sent to the plugin.
    ///
    /// The plugin is expected to clean up and close the pipe.
    pub fn shutdown(&self) {
        match self.0.lock() {
            Ok(mut inner) => {
                //FIXME: don't block here?
                inner.peer.send_rpc_notification("shutdown", &json!({}));
                // TODO: get rust plugin lib to respect shutdown msg
                if inner.description.name == "syntect" {
                    let _ = inner.process.kill();
                }
                eprintln!("waiting on process {}", inner.process.id());
                let exit_status = inner.process.wait();
                eprintln!("process ended {:?}", exit_status);
            }
            Err(_) => eprintln!("plugin mutex poisoned"),
        }
    }

    /// Returns `true` if this plugin has crashed.
    pub fn is_dead(&self) -> bool {
        self.1.load(Ordering::SeqCst)
    }

    /// Marks this plugin as having crashed.
    pub fn declare_dead(&mut self) {
        self.1.store(true, Ordering::SeqCst);
    }

    /// Returns this plugin instance's unique identifier.
    pub fn get_identifier(&self) -> PluginPid {
        self.0.lock().unwrap().identifier
    }

    /// Returns the plugin's name
    pub fn get_name(&self) -> String {
        self.0.lock().unwrap().description.name.clone()
    }
}


/// Starts a thread which collects editor updates and propagates them to plugins.
///
/// In addition to updates caused by user edits, updates can be caused by
/// plugin edits. These updates arrive asynchronously. After being applied to
/// the relevant buffer via an `Editor` instance, they need to be propagated
/// back out to all interested plugins.
///
/// In order to avoid additional complexity in the model graph (e.g. giving each
/// `Editor` a weak reference to the `PluginManager`) we instead give each
/// `Editor` a tx end of an `mpsc::channel`. As plugin updates are generated,
/// they are sent over this channel to a receiver running in another thread,
/// which forwards them to interested plugins.
pub fn start_update_thread(
    rx: mpsc::Receiver<(ViewIdentifier, PluginUpdate, usize, Timestamp)>,
    manager_ref: &PluginManagerRef)
{
    let manager_ref = manager_ref.clone();
    thread::spawn(move ||{
        loop {
            match rx.recv() {
                Ok((view_id, update, undo_group, timestamp)) => {
                    if let Some(err) = manager_ref.update_plugins(
                        view_id, update, undo_group, timestamp).err() {
                        eprintln!("error updating plugins {:?}", err);
                    }
                }
                Err(_) => break,
            }
        }
    });
}

/// Launches a plugin, associating it with a given view.
pub fn start_plugin_process<C>(manager_ref: &PluginManagerRef,
                          plugin_desc: &PluginDescription,
                          identifier: PluginPid,
                          completion: C)
    where C: FnOnce(Result<PluginRef, io::Error>) + Send + 'static
{

    let manager_ref = manager_ref.to_weak();
    let plugin_desc = plugin_desc.to_owned();

    thread::spawn(move || {
        eprintln!("starting plugin at path {:?}", &plugin_desc.exec_path);
        let child = ProcCommand::new(&plugin_desc.exec_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn();

        match child {
            Ok(mut child) => {
                let child_stdin = child.stdin.take().unwrap();
                let child_stdout = child.stdout.take().unwrap();
                let mut looper = RpcLoop::new(child_stdin);
                let peer: RpcPeer = Box::new(looper.get_raw_peer());
                peer.send_rpc_notification("ping", &Value::Array(Vec::new()));
                let plugin = Plugin {
                    peer: peer,
                    process: child,
                    manager: manager_ref,
                    description: plugin_desc,
                    identifier: identifier,
                };
                let mut plugin_ref = PluginRef(
                    Arc::new(Mutex::new(plugin)),
                    Arc::new(AtomicBool::new(false)));
                completion(Ok(plugin_ref.clone()));
                //TODO: we could be logging plugin exit results
                let _ = looper.mainloop(|| BufReader::new(child_stdout),
                                        &mut plugin_ref);
            }
            Err(err) => completion(Err(err)),
        }
    });
}
