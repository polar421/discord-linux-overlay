//! The control socket: lets the CLI (and therefore KDE shortcuts) drive a
//! running instance, and turns a second launch into "open settings".

use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use discord_overlay_core::control::{control_socket_path, Request, Response};
use tauri::{AppHandle, Manager};

use crate::commands;
use crate::state::AppState;

pub fn start_server(app: AppHandle) {
    let path = control_socket_path();

    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            log::warn!("could not create the control socket directory: {error}");
            return;
        }
    }

    // A stale socket from a crashed instance would block binding.
    let _ = std::fs::remove_file(&path);

    let spawn_result = thread::Builder::new()
        .name("control-socket".to_string())
        .spawn(move || {
            let listener = match UnixListener::bind(&path) {
                Ok(listener) => listener,
                Err(error) => {
                    log::warn!("control socket unavailable at {}: {error}", path.display());
                    return;
                }
            };

            for connection in listener.incoming() {
                let Ok(mut stream) = connection else {
                    continue;
                };

                let mut line = String::new();
                let read_result = {
                    let mut reader = match stream.try_clone() {
                        Ok(clone) => BufReader::new(clone),
                        Err(_) => continue,
                    };
                    reader.read_line(&mut line)
                };

                if read_result.is_err() {
                    continue;
                }

                let request: Request = match serde_json::from_str(line.trim()) {
                    Ok(request) => request,
                    Err(error) => {
                        let _ = write_response(
                            &mut stream,
                            Response::error(format!("invalid request: {error}")),
                        );
                        continue;
                    }
                };

                let (sender, receiver) = mpsc::channel();
                let app_for_task = app.clone();

                let dispatched = app.run_on_main_thread(move || {
                    let app_state = app_for_task.state::<AppState>();
                    if !matches!(request, Request::Status) {
                        log::info!("control request: {request:?}");
                    }
                    let _ = app_state.logs.lock().map(|mut logs| {
                        logs.push_back(format!("control request: {request:?}"));
                    });
                    let _ = sender.send(commands::apply_request(&app_for_task, request));
                });

                if dispatched.is_err() {
                    let _ = write_response(
                        &mut stream,
                        Response::error("the application is shutting down"),
                    );
                    continue;
                }

                let response = receiver
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap_or_else(|_| Response::error("the request timed out"));
                let _ = write_response(&mut stream, response);
            }
        });

    if let Err(error) = spawn_result {
        log::warn!("could not start the control socket thread: {error}");
    }
}

fn write_response(stream: &mut UnixStream, response: Response) -> std::io::Result<()> {
    let body = serde_json::to_string(&response).unwrap_or_else(|_| {
        "{\"ok\":false,\"message\":\"could not serialise the response\"}".to_string()
    });
    stream.write_all(body.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

/// Send a request to a running instance.
pub fn send_request(request: Request) -> Result<Response, String> {
    let path = control_socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|error| {
        format!(
            "is Discord Linux Overlay running? ({}: {error})",
            path.display()
        )
    })?;

    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;

    let body = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    stream
        .write_all(format!("{body}\n").as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;

    let mut reply = String::new();
    stream
        .read_to_string(&mut reply)
        .map_err(|error| error.to_string())?;

    serde_json::from_str(reply.trim())
        .map_err(|error| format!("invalid reply from the running instance: {error}"))
}

/// Read-only helper for tests and diagnostics.
#[allow(dead_code)]
fn socket_exists(path: &std::path::Path) -> bool {
    path.exists()
}
