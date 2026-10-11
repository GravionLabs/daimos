//! The resident process side of `daimos toggle | show | hide | quit`.

use std::sync::Mutex;

use daimos_core::ipc::{Endpoint, Request, Response, Server};
use tauri::{AppHandle, Manager, Wry};

use crate::window_control;

/// Holds the listener so it lives as long as the app and can be stopped on quit.
struct Listener(Mutex<Option<Server>>);

pub fn start(app: &AppHandle<Wry>) {
    let endpoint = match Endpoint::default_for_user() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            log::warn!("no socket for the daimos CLI: {error}");
            return;
        }
    };
    let handle = app.clone();
    let server = Server::start(endpoint, move |request| handle_request(&handle, request));
    match server {
        Ok(server) => {
            log::info!("listening on {:?}", server.endpoint());
            app.manage(Listener(Mutex::new(Some(server))));
        }
        Err(error) => log::warn!("cannot listen for the daimos CLI: {error}"),
    }
}

pub fn stop(app: &AppHandle<Wry>) {
    if let Some(listener) = app.try_state::<Listener>()
        && let Some(server) = listener.0.lock().expect("listener lock").take()
    {
        server.stop();
    }
}

fn handle_request(app: &AppHandle<Wry>, request: Request) -> Response {
    match request {
        Request::Toggle => window_control::toggle(app),
        Request::Show { query: Some(query) } => window_control::show_with_query(app, &query),
        Request::Show { query: None } => window_control::show(app),
        Request::Hide => window_control::hide(app),
        Request::Quit => {
            // Answer first: the exit stops this very listener.
            let app = app.clone();
            std::thread::spawn(move || app.exit(0));
        }
        Request::IndexRebuild => {
            return match index(app).map(|index| index.rebuild()) {
                Some(Ok(status)) => Response::Index(status),
                Some(Err(error)) => Response::Error { message: format!("reindex failed: {error}") },
                None => {
                    Response::Error { message: "the application index is not available".into() }
                }
            };
        }
        Request::IndexStatus => {
            return match index(app) {
                Some(index) => Response::Index(index.status()),
                None => {
                    Response::Error { message: "the application index is not available".into() }
                }
            };
        }
    }
    Response::Ok
}

fn index(app: &AppHandle<Wry>) -> Option<std::sync::Arc<daimos_core::index::IndexService>> {
    app.try_state::<std::sync::Arc<daimos_core::index::IndexService>>().map(|s| s.inner().clone())
}
