//! Native HTTP front end of the web app.
//!
//! `bone-infill serve` starts a small server bound to the loopback interface
//! and hands every request to [`crate::app::respond`]. The page does the
//! interaction and the 3D view; every computation happens here, natively and
//! on all cores, so regenerating after a parameter change costs what the
//! command line costs. It is meant for one person on their own machine.

use crate::app::{Reply, Shared, respond, safe_name};
use anyhow::{Result, anyhow};
use std::io::Read;
use std::sync::Arc;
use tiny_http::{Header, Method, Response, Server, StatusCode};

/// Largest upload accepted (bytes).
const MAX_UPLOAD: u64 = 2 << 30;
const WORKERS: usize = 4;

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header names and sanitised values are valid")
}

fn handle(mut request: tiny_http::Request, shared: &Shared) {
    let method = match request.method() {
        Method::Get => "GET",
        Method::Post => "POST",
        _ => "OTHER",
    };
    let url = request.url().to_string();
    let mut body = Vec::new();
    let reply = if request.body_length().is_some_and(|n| n as u64 > MAX_UPLOAD) {
        Reply::error(413, "the file is too large")
    } else {
        match request.as_reader().take(MAX_UPLOAD).read_to_end(&mut body) {
            Ok(_) => respond(shared, method, &url, &body),
            Err(e) => Reply::error(400, format!("could not read the request: {e}")),
        }
    };
    let mut response = Response::from_data(reply.body)
        .with_status_code(StatusCode(reply.status))
        .with_header(header("Content-Type", reply.content_type))
        .with_header(header("Cache-Control", "no-store"));
    if let Some(name) = reply.download {
        response.add_header(header("Content-Disposition", &format!("attachment; filename=\"{}\"", safe_name(&name))));
    }
    // The browser may have gone away; nothing useful to do about it.
    let _ = request.respond(response);
}

fn open_in_browser(url: &str) {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if result.is_err() {
        println!("could not open a browser; open the address above by hand");
    }
}

/// Runs the web app until the process is stopped.
pub fn serve(port: u16, open_browser: bool) -> Result<()> {
    let address = format!("127.0.0.1:{port}");
    let server = Server::http(&address)
        .map_err(|e| anyhow!("could not listen on {address}: {e} (is another instance running? try --port)"))?;
    let url = format!("http://{address}/");
    println!("bone-infill is running at {url}");
    println!("press Ctrl+C to stop");
    if open_browser {
        open_in_browser(&url);
    }
    let server = Arc::new(server);
    let shared = Arc::new(Shared::default());
    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            let (server, shared) = (Arc::clone(&server), Arc::clone(&shared));
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    handle(request, &shared);
                }
            })
        })
        .collect();
    for worker in workers {
        let _ = worker.join();
    }
    Ok(())
}
