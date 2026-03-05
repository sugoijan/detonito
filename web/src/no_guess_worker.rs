use crate::utils::console_error;
use detonito_core as game;
use game::NoGuessLayoutGenerator;
use gloo::utils::document;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    Blob, BlobPropertyBag, DedicatedWorkerGlobalScope, MessageEvent, Url, Worker, WorkerOptions,
    WorkerType,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct NoGuessGenRequest {
    pub generation_id: u64,
    pub seed: u64,
    pub first_move: game::Coord2,
    pub config: game::GameConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct NoGuessGenSummary {
    pub attempts: usize,
    pub backtracks: usize,
    pub max_depth_reached: usize,
    pub elapsed_micros: u128,
    pub succeeded: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct NoGuessGenResponse {
    pub generation_id: u64,
    pub layout: game::MineLayout,
    pub summary: NoGuessGenSummary,
}

pub(crate) struct NoGuessWorkerBridge {
    worker: Worker,
    state: Rc<RefCell<BridgeState>>,
    _onmessage: Closure<dyn FnMut(MessageEvent)>,
    _onmessageerror: Closure<dyn FnMut(JsValue)>,
    _onerror: Closure<dyn FnMut(JsValue)>,
}

impl core::fmt::Debug for NoGuessWorkerBridge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NoGuessWorkerBridge")
    }
}

impl NoGuessWorkerBridge {
    pub fn send(&self, req: NoGuessGenRequest) -> bool {
        let mut state = self.state.borrow_mut();
        match &mut *state {
            BridgeState::Initializing(queue) => {
                queue.push(req);
                true
            }
            BridgeState::Ready => post_request(&self.worker, req),
            BridgeState::Failed => {
                console_error("No-guess worker bridge is in failed state");
                false
            }
        }
    }

    pub fn terminate(self) {
        self.worker.terminate();
    }
}

pub(crate) fn register_worker() {
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let worker_scope = scope.clone();

    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(payload) = event.data().as_string() else {
            console_error("No-guess worker received a non-string payload");
            return;
        };

        let req: NoGuessGenRequest = match serde_json::from_str(&payload) {
            Ok(req) => req,
            Err(err) => {
                console_error(&format!(
                    "No-guess worker failed to parse request payload: {err}"
                ));
                return;
            }
        };

        let (layout, stats) =
            NoGuessLayoutGenerator::new(req.seed, req.first_move).generate_with_stats(req.config);

        let summary = NoGuessGenSummary {
            attempts: stats.attempts,
            backtracks: stats.backtracks,
            max_depth_reached: stats.max_depth_reached,
            elapsed_micros: stats.elapsed_micros,
            succeeded: stats.succeeded,
        };

        let response = NoGuessGenResponse {
            generation_id: req.generation_id,
            layout,
            summary,
        };

        let response_payload = match serde_json::to_string(&response) {
            Ok(payload) => payload,
            Err(err) => {
                console_error(&format!(
                    "No-guess worker failed to serialize response: {err}"
                ));
                return;
            }
        };

        if let Err(err) = worker_scope.post_message(&JsValue::from_str(&response_payload)) {
            console_error(&format!("No-guess worker failed to post response: {err:?}"));
        }
    });

    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();
}

pub(crate) fn spawn_bridge<F>(mut callback: F) -> Option<NoGuessWorkerBridge>
where
    F: FnMut(NoGuessGenResponse) + 'static,
{
    let worker_path = resolve_worker_path().and_then(|path| to_absolute_url(&path))?;
    let loader_path = create_loader_script_url(&worker_path)?;
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options(&loader_path, &options).ok()?;
    Url::revoke_object_url(&loader_path).ok();

    let state = Rc::new(RefCell::new(BridgeState::Initializing(Vec::new())));
    let worker_for_messages = worker.clone();
    let state_for_messages = state.clone();

    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(payload) = event.data().as_string() else {
            console_error("No-guess worker bridge received a non-string payload");
            return;
        };

        if payload == WORKER_READY_MESSAGE {
            let queued = match &mut *state_for_messages.borrow_mut() {
                BridgeState::Initializing(queue) => core::mem::take(queue),
                BridgeState::Ready => Vec::new(),
                BridgeState::Failed => Vec::new(),
            };

            *state_for_messages.borrow_mut() = BridgeState::Ready;
            for req in queued {
                post_request(&worker_for_messages, req);
            }
            return;
        }

        if let Some(err) = payload.strip_prefix(WORKER_ERROR_PREFIX) {
            *state_for_messages.borrow_mut() = BridgeState::Failed;
            console_error(&format!("No-guess worker failed to initialize: {err}"));
            return;
        }

        match serde_json::from_str::<NoGuessGenResponse>(&payload) {
            Ok(response) => callback(response),
            Err(err) => console_error(&format!(
                "No-guess worker bridge failed to parse response: {err}; payload={payload}"
            )),
        }
    });
    worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

    let state_for_message_error = state.clone();
    let onmessageerror = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
        *state_for_message_error.borrow_mut() = BridgeState::Failed;
        console_error("No-guess worker message channel error");
    });
    worker.set_onmessageerror(Some(onmessageerror.as_ref().unchecked_ref()));

    let state_for_error = state.clone();
    let onerror = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        *state_for_error.borrow_mut() = BridgeState::Failed;
        console_error(&format!(
            "No-guess worker runtime error: {}",
            js_value_to_string(&event)
        ));
    });
    worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));

    Some(NoGuessWorkerBridge {
        worker,
        state,
        _onmessage: onmessage,
        _onmessageerror: onmessageerror,
        _onerror: onerror,
    })
}

#[derive(Debug)]
enum BridgeState {
    Initializing(Vec<NoGuessGenRequest>),
    Ready,
    Failed,
}

const WORKER_READY_MESSAGE: &str = "__detonito_no_guess_worker_ready__";
const WORKER_ERROR_PREFIX: &str = "__detonito_no_guess_worker_error__:";

fn post_request(worker: &Worker, req: NoGuessGenRequest) -> bool {
    let payload = match serde_json::to_string(&req) {
        Ok(payload) => payload,
        Err(err) => {
            console_error(&format!(
                "Failed to serialize no-guess generation request: {err}"
            ));
            return false;
        }
    };

    if let Err(err) = worker.post_message(&JsValue::from_str(&payload)) {
        console_error(&format!(
            "Failed to post no-guess generation request to worker: {err:?}"
        ));
        return false;
    }

    true
}

fn create_loader_script_url(worker_path: &str) -> Option<String> {
    let wasm_path = worker_path.strip_suffix(".js")?.to_owned() + "_bg.wasm";
    let worker_path_js = serde_json::to_string(worker_path).ok()?;
    let wasm_path_js = serde_json::to_string(&wasm_path).ok()?;
    let loader = format!(
        "import init from {worker_path_js};\n(async () => {{\n  try {{\n    await init({{ module_or_path: {wasm_path_js} }});\n    self.postMessage(\"{WORKER_READY_MESSAGE}\");\n  }} catch (err) {{\n    const msg = err && err.stack ? err.stack : String(err);\n    self.postMessage(\"{WORKER_ERROR_PREFIX}\" + msg);\n  }}\n}})();\n"
    );
    let source = js_sys::Array::new();
    source.push(&JsValue::from_str(&loader));

    let options = BlobPropertyBag::new();
    options.set_type("text/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&source, &options).ok()?;
    Url::create_object_url_with_blob(&blob).ok()
}

fn to_absolute_url(path: &str) -> Option<String> {
    let location = web_sys::window()?.location().href().ok()?;
    let url = Url::new_with_base(path, &location).ok()?;
    Some(url.href())
}

fn js_value_to_string(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::JSON::stringify(value)
                .ok()
                .and_then(|s| s.as_string())
        })
        .unwrap_or_else(|| format!("{value:?}"))
}

fn resolve_worker_path() -> Option<String> {
    let doc = document();
    let selectors = [
        "link[rel='modulepreload'][href*='detonito-webapp'][href$='.js']",
        "link[rel='modulepreload'][href$='.js']",
        "script[src*='detonito-webapp'][src$='.js']",
        "script[src$='.js']",
    ];

    for selector in selectors {
        if let Some(node) = doc.query_selector(selector).ok().flatten() {
            if let Some(path) = node
                .get_attribute("href")
                .or_else(|| node.get_attribute("src"))
            {
                return Some(path);
            }
        }
    }

    None
}
