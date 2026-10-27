//! The tools an agent calls, from several providers: the built-in ones,
//! then those a host adds, such as the studio's live tools.

use std::panic::AssertUnwindSafe;

use serde_json::Value;

use crate::Builtin;

/// A provider of tools.
pub trait Tools: Send + Sync {
    /// The tools, as `tools/list` gives each one: `{name, title,
    /// description, inputSchema}`.
    fn list(&self) -> Vec<Value>;

    /// Runs the tool `name` with its arguments, an object: its content,
    /// MCP content items (`{"type": "text", "text"}` or `{"type": "image",
    /// "data", "mimeType"}`), or the error the agent reads. `None` when
    /// this provider has no such tool.
    fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>>;
}

/// Providers of tools, asked in order; no two give a tool the same name.
pub struct Registry {
    providers: Vec<Box<dyn Tools>>,
}

impl Registry {
    /// No tools.
    pub fn new() -> Registry {
        Registry { providers: Vec::new() }
    }

    /// The built-in tools, those `c2f mcp` serves.
    pub fn builtin() -> Registry {
        Registry { providers: vec![Box::new(Builtin)] }
    }

    /// This registry with the tools of `provider` after its own; the error
    /// names a tool it has already.
    pub fn with(mut self, provider: Box<dyn Tools>) -> Result<Registry, String> {
        let names = self.names();
        if let Some(name) = provider.list().iter().filter_map(name).find(|name| names.contains(name)) {
            return Err(format!("two tools are named `{name}`"));
        }
        self.providers.push(provider);
        Ok(self)
    }

    /// Every tool, provider after provider.
    pub fn list(&self) -> Vec<Value> {
        self.providers.iter().flat_map(|provider| provider.list()).collect()
    }

    /// The tools' names, in the order of [`Registry::list`].
    pub fn names(&self) -> Vec<String> {
        self.list().iter().filter_map(name).collect()
    }

    /// Runs the tool `name`, as [`Tools::call`] does; `None` when no
    /// provider has it. A panic in a tool ends this call, not its caller.
    pub fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
        self.providers.iter().find_map(|provider| {
            std::panic::catch_unwind(AssertUnwindSafe(|| provider.call(name, args)))
                .unwrap_or_else(|_| Some(Err(format!("internal error: `{name}` panicked (the server's log says where)"))))
        })
    }
}

impl Default for Registry {
    fn default() -> Registry {
        Registry::new()
    }
}

fn name(tool: &Value) -> Option<String> {
    tool.get("name").and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A host's tool, `echo`, or a tool named as a built-in one.
    struct Echo(&'static str);

    impl Tools for Echo {
        fn list(&self) -> Vec<Value> {
            vec![json!({ "name": self.0, "description": "Says its `text` back.", "inputSchema": { "type": "object" } })]
        }

        fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
            (name == self.0).then(|| Ok(vec![json!({ "type": "text", "text": args["text"] })]))
        }
    }

    #[test]
    fn the_builtin_tools_render_code() {
        let registry = Registry::builtin();
        assert_eq!(registry.names().len(), 20);
        assert!(registry.names().iter().any(|name| name == "render"));
        let content = registry.call("render", &json!({ "code": "cube(10);" })).expect("a tool").expect("a report");
        let report: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(report["model"], "3D");
        assert_eq!(report["volume"], 1000.0);
        let failed = registry.call("render", &json!({ "code": "cube(" })).expect("a tool");
        assert!(failed.unwrap_err().contains("\"line\": 1"));
    }

    #[test]
    fn a_host_adds_its_tools_after_the_builtin_ones() {
        let registry = Registry::builtin().with(Box::new(Echo("echo"))).unwrap();
        assert_eq!(registry.names().last().map(String::as_str), Some("echo"));
        assert_eq!(registry.list().len(), 21);
        let content = registry.call("echo", &json!({ "text": "hello" })).unwrap().unwrap();
        assert_eq!(content, vec![json!({ "type": "text", "text": "hello" })]);
    }

    #[test]
    fn a_tool_named_twice_is_refused() {
        let err = Registry::builtin().with(Box::new(Echo("render"))).err().expect("refused");
        assert!(err.contains("`render`"), "{err}");
        let err = Registry::new().with(Box::new(Echo("echo"))).unwrap().with(Box::new(Echo("echo"))).err();
        assert!(err.is_some());
    }

    #[test]
    fn an_unknown_tool_is_none() {
        assert!(Registry::builtin().call("nothing", &json!({})).is_none());
        assert!(Registry::new().call("render", &json!({ "code": "cube(1);" })).is_none());
    }

    #[test]
    fn a_panic_is_the_call_s_error() {
        struct Panics;
        impl Tools for Panics {
            fn list(&self) -> Vec<Value> {
                vec![json!({ "name": "panics" })]
            }
            fn call(&self, _: &str, _: &Value) -> Option<Result<Vec<Value>, String>> {
                panic!("on purpose")
            }
        }
        let registry = Registry::new().with(Box::new(Panics)).unwrap();
        assert!(registry.call("panics", &json!({})).unwrap().unwrap_err().contains("panicked"));
    }
}
