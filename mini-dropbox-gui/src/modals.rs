use std::sync::Arc;

use anyhow::anyhow;
use anyhow::Result;
use egui::{Id, Modal, Ui};
use reqwest::Client;
use tokio::sync::oneshot;
use tokio::sync::oneshot::{error::TryRecvError, Sender};

use crate::responses::response_to_message;
use crate::{
    requests::{self, Register},
    MyApp,
};

impl MyApp {
    pub fn show_username_modal(&mut self, ui: &Ui) {
        if !self.should_open_username_modal {
            return;
        }

        let modal = Modal::new(Id::new("Username modal"));
        let a = String::from("bitch");
        modal.show(ui.ctx(), |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 5.0);

            ui.vertical_centered(|ui| {
                ui.label("Enter a username for this device");
            });

            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Username");
                let text_edit = egui::TextEdit::singleline(&mut self.username)
                    .hint_text("Someone's laptop")
                    .desired_width(150.0);
                let resp = ui.add(text_edit);

                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if !self.username.trim().is_empty() {
                        let (sender, receiver) = oneshot::channel::<Result<String>>();

                        let client = self.reqwest_client.clone();
                        let username = self.username.clone();

                        tokio::spawn(
                            async move { create_username(client, username, sender).await },
                        );
                        self.username_rx = Some(receiver);
                    }
                }
            });

            if !self.create_username_message.is_empty() {
                ui.label(self.create_username_message.clone());
            }
        });
    }

    pub fn poll_username_response(&mut self) {
        let Some(rx) = &mut self.username_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(result) => {
                self.username_rx = None;
                match result {
                    Ok(success_msg) => {
                        self.create_username_message = success_msg;
                        self.should_open_username_modal = false;
                    }
                    Err(err_msg) => {
                        self.create_username_message = err_msg.to_string();
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Closed) => {
                self.username_rx = None;
                self.create_username_message = "Connection closed before response".into();
            }
        }
    }
}

async fn create_username(client: Arc<Client>, username: String, sender: Sender<Result<String>>) {
    match client
        .post("http://localhost:3000/register")
        .headers(requests::construct_headers())
        .body(Register { username })
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let message = response_to_message(r).await;
            let _ = sender.send(Ok(message));
        }
        Ok(r) => {
            let message = response_to_message(r).await;
            let _ = sender.send(Err(anyhow!(message)));
        }
        Err(e) => {
            let _ = sender.send(Err(anyhow!(e.to_string())));
        }
    }
}
