//! `/host <url>` — point the Ollama client at a different server and
//! refresh the model list in the background.

use tokio::sync::mpsc;

use crate::ollama::Client;

use super::super::{App, StreamMsg};

impl App {
    pub(in crate::app) fn switch_host(
        &mut self,
        url: String,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        let url = url.trim();
        if url.is_empty() {
            self.push_info(format!(
                "Current host: {}\nUsage: /host <url>",
                self.client.base
            ));
            return;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.push_info(format!(
                "Host must start with http:// or https://. Got: {url}"
            ));
            self.status = "Invalid host URL".into();
            return;
        }
        self.client = Client::new(url.to_string());
        self.push_info(format!(
            "Host set to {}. Refreshing models…",
            self.client.base
        ));
        self.status = "Refreshing models…".into();
        let client = self.client.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            match client.list_models().await {
                Ok(models) => {
                    let _ = tx.send(StreamMsg::Models {
                        models,
                        base: client.base.clone(),
                    });
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("list models: {e}")));
                }
            }
        });
    }
}
