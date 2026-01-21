use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message};
use wasm_bindgen_futures::spawn_local;
use web_sys::console;

const WS_URL: &str = "wss://fling-server.fly.dev/ws";

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    spawn_local(async {
        let document = web_sys::window()
            .expect("no window")
            .document()
            .expect("no document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("failed to get canvas id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("canvas id isn't an HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(MyApp::default()))),
            )
            .await;

        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => loading_text.remove(),
                Err(e) => {
                    panic!("app crashed {e:?}");
                }
            }
        }
    });
}

struct MyApp {
    name: String,
    age: u32,
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            name: "Arthur".to_owned(),
            age: 42,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(&ctx, |ui| {
            ui.heading("My egui Application");
            ui.horizontal(|ui| {
                let name_label = ui.label("Your name: ");
                ui.text_edit_singleline(&mut self.name)
                    .labelled_by(name_label.id);
            });
            ui.add(egui::Slider::new(&mut self.age, 0..=120).text("age"));
            if ui.button("Increment").clicked() {
                self.age += 1;
            }
            ui.label(format!("Hello '{}', age {}", self.name, self.age));
            console::log_1(&"test".into());

            let mut ws = WebSocket::open(WS_URL).unwrap();
            let (mut write, mut read) = ws.split();
            spawn_local(async move {
                while let Some(ws_msg) = read.next().await {
                    match ws_msg {
                        Ok(Message::Text(msg)) => {
                            console::log_1(&msg.into());
                        }
                        Ok(Message::Bytes(b)) => {}
                        Err(e) => {
                            console::log_1(&e.to_string().into());
                        }
                    }
                }
            });

            spawn_local(async move {
                write
                    .send(Message::Text(
                        WebSocketMessage::Register("Dylan".into()).to_json(),
                    ))
                    .await
                    .unwrap();
            });
        });
    }
}
