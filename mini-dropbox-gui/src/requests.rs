use reqwest::{
    header::{HeaderMap, HeaderValue, CONTENT_TYPE},
    Body,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct Register {
    pub username: String,
}

impl From<Register> for Body {
    fn from(value: Register) -> Self {
        let json = serde_json::to_string(&value).unwrap();
        Body::from(json)
    }
}

pub fn construct_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}
