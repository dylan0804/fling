use serde::Deserialize;

#[derive(Deserialize)]
pub struct Response {
    pub message: String,
}

pub async fn response_to_message(r: reqwest::Response) -> String {
    let status = r.status();

    match r.text().await {
        Ok(body) => match serde_json::from_str::<Response>(&body) {
            Ok(response) => response.message,
            Err(e) => format!("Error parsing response: {}", e.to_string()),
        },
        Err(e) => {
            format!("Failed to read response body: {}", e.to_string())
        }
    }
}
