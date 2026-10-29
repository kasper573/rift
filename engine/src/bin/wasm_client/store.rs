use std::sync::Arc;

use engine::core::content::store::ContentStore;
use engine::core::platform::LocalFuture;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

const CACHE: &str = "rift-content";
const KEY_PREFIX: &str = "/rift-content/";

pub struct WebStore;

impl ContentStore for WebStore {
    fn list(&self) -> LocalFuture<Vec<String>> {
        Box::pin(async {
            let Some(cache) = open().await else {
                return Vec::new();
            };
            let Ok(keys) = JsFuture::from(cache.keys()).await else {
                return Vec::new();
            };
            let Ok(keys) = keys.dyn_into::<js_sys::Array>() else {
                return Vec::new();
            };
            keys.iter()
                .filter_map(|request| request.dyn_into::<web_sys::Request>().ok())
                .filter_map(|request| hash_of(&request.url()))
                .collect()
        })
    }

    fn read(&self, hash: &str) -> LocalFuture<Option<Vec<u8>>> {
        let key = key(hash);
        Box::pin(async move {
            let cache = open().await?;
            let matched = JsFuture::from(cache.match_with_str(&key)).await.ok()?;
            let response = matched.dyn_into::<web_sys::Response>().ok()?;
            let buffer = JsFuture::from(response.array_buffer().ok()?).await.ok()?;
            Some(js_sys::Uint8Array::new(&buffer).to_vec())
        })
    }

    fn write(&self, hash: &str, bytes: Arc<[u8]>) -> LocalFuture<Result<(), String>> {
        let key = key(hash);
        Box::pin(async move {
            let cache = open().await.ok_or("no cache storage")?;
            let array = js_sys::Uint8Array::from(&bytes[..]);
            let response = web_sys::Response::new_with_opt_buffer_source(Some(&array))
                .map_err(|error| format!("{error:?}"))?;
            JsFuture::from(cache.put_with_str(&key, &response))
                .await
                .map(|_| ())
                .map_err(|error| format!("{error:?}"))
        })
    }

    fn remove(&self, hash: &str) -> LocalFuture<()> {
        let key = key(hash);
        Box::pin(async move {
            if let Some(cache) = open().await {
                let _ = JsFuture::from(cache.delete_with_str(&key)).await;
            }
        })
    }
}

async fn open() -> Option<web_sys::Cache> {
    let caches = web_sys::window()?.caches().ok()?;
    JsFuture::from(caches.open(CACHE))
        .await
        .ok()?
        .dyn_into::<web_sys::Cache>()
        .ok()
}

fn key(hash: &str) -> String {
    format!("{KEY_PREFIX}{hash}")
}

fn hash_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once(KEY_PREFIX)?;
    Some(rest.to_owned())
}
