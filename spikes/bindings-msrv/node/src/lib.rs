use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi]
pub fn echo(text: String) -> serde_json::Value {
    serde_json::json!({ "text": text })
}

#[napi]
pub async fn delayed(text: String) -> Result<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = tx.send(text);
    });
    rx.await.map_err(|e| Error::from_reason(e.to_string()))
}
