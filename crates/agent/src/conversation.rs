//! A conversation with an agent, whatever runs it: the studio's chat panel
//! sends the user's messages to an [`Engine`] (Claude Code in a
//! subprocess, or the Anthropic API) and shows the [`Event`]s it streams
//! back.

use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What an engine streams back during a turn, in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// More of the assistant's reply, to append to what came before.
    Text { delta: String },
    /// The agent calls a tool with `input`, its arguments.
    ToolCall { id: String, name: String, input: Value },
    /// What the call `id` gave back: MCP content items (text or image), or
    /// the error the agent reads.
    ToolResult { id: String, content: Vec<Value>, is_error: bool },
    /// The turn is over: what it cost when the engine says, and the
    /// session to resume, if any.
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// The turn stops on an error.
    Error { message: String },
}

/// The tokens a turn took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// What the user sends: their text, and what they point the agent at.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

impl UserMessage {
    /// Text alone.
    pub fn text(text: impl Into<String>) -> UserMessage {
        UserMessage { text: text.into(), attachments: Vec::new() }
    }
}

/// Something sent with a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Attachment {
    /// A picture, as PNG bytes: the view as the user sees it.
    Image { png: Vec<u8> },
    /// What the user has selected, as text: a part (`label` its name) or a
    /// range of code (`label` its file and lines).
    Context { label: String, text: String },
}

/// What runs the agent: it takes the user's messages and streams back
/// what happens.
pub trait Engine: Send {
    /// Starts a turn with `message`; its events go to `events`, the last
    /// one [`Event::Done`] or [`Event::Error`]. It may return before the
    /// turn is over, the events coming from another thread; the error says
    /// why the turn could not start.
    fn send(&mut self, message: UserMessage, events: Sender<Event>) -> Result<(), String>;

    /// Stops the turn under way, if any.
    fn stop(&mut self);

    /// The engine's name, to show the user.
    fn name(&self) -> &str;
}

/// An engine that plays scripted turns, for tests: each message sent gets
/// the next turn's events.
#[derive(Debug, Default)]
pub struct FakeEngine {
    turns: Vec<Vec<Event>>,
    received: Vec<UserMessage>,
    stops: usize,
}

impl FakeEngine {
    /// An engine that answers its messages with `turns`, in order.
    pub fn new(turns: Vec<Vec<Event>>) -> FakeEngine {
        FakeEngine { turns, received: Vec::new(), stops: 0 }
    }

    /// The messages sent to it, in order.
    pub fn received(&self) -> &[UserMessage] {
        &self.received
    }

    /// How many times it was stopped.
    pub fn stops(&self) -> usize {
        self.stops
    }
}

impl Engine for FakeEngine {
    fn send(&mut self, message: UserMessage, events: Sender<Event>) -> Result<(), String> {
        if self.turns.is_empty() {
            return Err("the fake engine has no turn left".into());
        }
        self.received.push(message);
        for event in self.turns.remove(0) {
            // A listener gone stops nothing: the turn is played anyway.
            let _ = events.send(event);
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.stops += 1;
    }

    fn name(&self) -> &str {
        "fake"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use serde_json::json;

    use super::*;

    #[test]
    fn a_fake_engine_plays_its_turns() {
        let turn = vec![
            Event::Text { delta: "Here".into() },
            Event::ToolCall { id: "1".into(), name: "render".into(), input: json!({ "code": "cube(1);" }) },
            Event::ToolResult { id: "1".into(), content: vec![json!({ "type": "text", "text": "{}" })], is_error: false },
            Event::Done { usage: Some(Usage { input_tokens: 10, output_tokens: 5 }), cost_usd: None, session_id: None },
        ];
        let mut engine: Box<dyn Engine> = Box::new(FakeEngine::new(vec![turn.clone()]));
        let (sender, receiver) = mpsc::channel();
        engine.send(UserMessage::text("a cube"), sender.clone()).unwrap();
        assert_eq!(receiver.try_iter().collect::<Vec<_>>(), turn);
        assert!(engine.send(UserMessage::text("again"), sender).is_err());
        engine.stop();
        assert_eq!(engine.name(), "fake");
    }

    #[test]
    fn events_and_messages_read_as_json() {
        let done = Event::Done { usage: None, cost_usd: Some(0.01), session_id: Some("s".into()) };
        assert_eq!(serde_json::to_value(&done).unwrap(), json!({ "type": "done", "cost_usd": 0.01, "session_id": "s" }));
        let message = UserMessage {
            text: "make it wider".into(),
            attachments: vec![
                Attachment::Image { png: vec![137, 80] },
                Attachment::Context { label: "box.c2f:3-5".into(), text: "cube(10);".into() },
            ],
        };
        let back: UserMessage = serde_json::from_value(serde_json::to_value(&message).unwrap()).unwrap();
        assert_eq!(back, message);
        let text: UserMessage = serde_json::from_value(json!({ "text": "hi" })).unwrap();
        assert_eq!(text, UserMessage::text("hi"));
    }
}
