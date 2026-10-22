//! `c2f mcp` as an agent sees it: the protocol, then each tool.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    fn start() -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_c2f"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("starting c2f mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server { child, stdin, stdout, next_id: 1 }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn reply(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|err| panic!("{err}: {line:?}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let reply = self.reply();
        assert_eq!((&reply["jsonrpc"], &reply["id"]), (&json!("2.0"), &json!(id)), "{reply}");
        reply
    }

    /// Calls a tool: whether it failed, and its content.
    fn call(&mut self, tool: &str, arguments: Value) -> (bool, Vec<Value>) {
        let reply = self.request("tools/call", json!({ "name": tool, "arguments": arguments }));
        let result = &reply["result"];
        (result["isError"].as_bool().expect("isError"), result["content"].as_array().expect("content").clone())
    }

    /// Calls a tool that answers with one JSON text, and reads it.
    fn report(&mut self, tool: &str, arguments: Value) -> (bool, Value) {
        let (is_error, content) = self.call(tool, arguments);
        let text = content.last().and_then(|c| c["text"].as_str()).expect("a text");
        (is_error, serde_json::from_str(text).unwrap_or_else(|_| json!(text)))
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("c2f-mcp-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn protocol() {
    let mut server = Server::start();
    let init = server.request(
        "initialize",
        json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "code2form");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    // A notification gets no answer: the next line answers the ping.
    server.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    assert_eq!(server.request("ping", json!({}))["result"], json!({}));

    let list = server.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["render", "parameters", "image", "export"]);
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(tool["inputSchema"]["properties"]["code"].is_object(), "{tool}");
    }

    assert_eq!(server.request("resources/list", json!({}))["error"]["code"], -32601);
    assert_eq!(server.request("tools/call", json!({ "name": "nope" }))["error"]["code"], -32602);
    server.stdin.write_all(b"{not json\n").unwrap();
    assert_eq!(server.reply()["error"]["code"], -32700);

    // An unknown version gets the latest.
    let init = server.request("initialize", json!({ "protocolVersion": "1999-01-01" }));
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

    drop(server.stdin);
    assert!(server.child.wait().unwrap().success());
}

#[test]
fn render_reports_errors_echo_and_stats() {
    let mut server = Server::start();
    let (is_error, report) = server.report("render", json!({ "code": "cube(10);\nsphere(;\n" }));
    assert!(is_error);
    assert_eq!((&report["line"], &report["column"]), (&json!(2), &json!(8)), "{report}");
    assert!(report["error"].as_str().unwrap().contains("expected"), "{report}");

    let (is_error, report) = server.report("render", json!({ "code": "assert(1 > 2, \"too small\");" }));
    assert!(is_error);
    assert!(report["error"].as_str().unwrap().contains("too small"), "{report}");

    // Includes resolve next to the file.
    let dir = temp_dir("render");
    std::fs::write(dir.join("size.scad"), "size = [10, 20, 30];\n").unwrap();
    std::fs::write(dir.join("part.scad"), "include <size.scad>\necho(size);\ncube(size);\nundefined_module();\n").unwrap();
    let (is_error, report) = server.report("render", json!({ "path": dir.join("part.scad") }));
    assert!(!is_error, "{report}");
    assert_eq!(report["echo"], json!(["ECHO: [10, 20, 30]"]));
    assert!(report["warnings"][0].as_str().unwrap().contains("undefined_module"), "{report}");
    assert_eq!((&report["model"], &report["triangles"], &report["volume"]), (&json!("3D"), &json!(12), &json!(6000.0)));
    assert_eq!(report["bounding_box"], json!({ "min": [0.0, 0.0, 0.0], "max": [10.0, 20.0, 30.0], "size": [10.0, 20.0, 30.0] }));

    // With `code`, from the working directory.
    let (_, report) = server.report("render", json!({ "code": "include <size.scad>\ncube(size);", "working_dir": dir }));
    assert_eq!(report["volume"], 6000.0, "{report}");

    let (is_error, report) = server.report("render", json!({ "code": "square([3, 4]);" }));
    assert!(!is_error);
    assert_eq!((&report["model"], &report["area"]), (&json!("2D"), &json!(12.0)), "{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

const CUSTOMIZED: &str = "\
width = 10; // [5:50]
/* [Look] */
// What it says
label = \"A\"; // [A, B]
size = [1, 2]; // [0:10]
echo(label, size);
cube([width, 1, 1]);
";

#[test]
fn parameters_are_listed_and_set() {
    let mut server = Server::start();
    let (is_error, report) = server.report("parameters", json!({ "code": CUSTOMIZED }));
    assert!(!is_error);
    let parameters = report["parameters"].as_array().unwrap();
    let names: Vec<&str> = parameters.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["width", "label", "size"]);
    assert_eq!(parameters[0]["widget"], json!({ "type": "slider", "min": 5.0, "max": 50.0, "step": 1.0 }));
    assert_eq!((&parameters[1]["group"], &parameters[1]["description"]), (&json!("Look"), &json!("What it says")));
    assert_eq!(parameters[1]["widget"]["options"][1]["value"], "B");
    assert_eq!(parameters[2]["default"], json!([1.0, 2.0]));

    let values = json!({ "width": 25, "label": "B", "size": [3, 4] });
    let (is_error, report) = server.report("render", json!({ "code": CUSTOMIZED, "parameters": values }));
    assert!(!is_error, "{report}");
    assert_eq!(report["volume"], 25.0);
    assert_eq!(report["echo"], json!(["ECHO: \"B\", [3, 4]"]));

    let (is_error, content) = server.call("render", json!({ "code": CUSTOMIZED, "parameters": { "width": "wide" } }));
    assert!(is_error);
    assert!(content[0]["text"].as_str().unwrap().contains("width"), "{content:?}");
    let (is_error, _) = server.call("render", json!({ "code": CUSTOMIZED, "parameters": { "depth": 1 } }));
    assert!(is_error);
}

#[test]
fn image_is_a_png_of_the_model() {
    let mut server = Server::start();
    let code = "color(\"red\") cube(10, center = true);";
    let (is_error, content) = server.call("image", json!({ "code": code, "view": "front", "width": 200, "height": 150 }));
    assert!(!is_error, "{content:?}");
    assert_eq!((&content[0]["type"], &content[0]["mimeType"]), (&json!("image"), &json!("image/png")));
    let png = base64_decode(content[0]["data"].as_str().unwrap());
    let (width, height, rgb) = decode_png(&png);
    assert_eq!((width, height), (200, 150));
    // The background in the corner, the cube's red face in the middle.
    let pixel = |x: usize, y: usize| &rgb[(y * width + x) * 3..][..3];
    let (corner, middle) = (pixel(0, 0), pixel(100, 75));
    assert!(corner.iter().all(|&c| c < 80), "{corner:?}");
    assert!(middle[0] > 150 && middle[1] < 40 && middle[2] < 40, "{middle:?}");
    // The render report comes with it.
    let report: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["volume"], 1000.0);

    // The default view and size, every view, and a 2D model.
    let (_, content) = server.call("image", json!({ "code": code }));
    let (width, height, _) = decode_png(&base64_decode(content[0]["data"].as_str().unwrap()));
    assert_eq!((width, height), (800, 600));
    for view in ["diagonal", "isometric", "top", "bottom", "front", "back", "left", "right"] {
        let (is_error, content) = server.call("image", json!({ "code": code, "view": view, "width": 40, "height": 30 }));
        assert!(!is_error, "{view}");
        assert_eq!(decode_png(&base64_decode(content[0]["data"].as_str().unwrap())).0, 40, "{view}");
    }
    let (is_error, _) = server.call("image", json!({ "code": "circle(5);", "view": "top", "width": 64, "height": 64 }));
    assert!(!is_error);

    let (is_error, content) = server.call("image", json!({ "code": "echo(1);" }));
    assert!(is_error);
    assert!(content[0]["text"].as_str().unwrap().contains("empty"), "{content:?}");
    let (is_error, _) = server.call("image", json!({ "code": code, "view": "under" }));
    assert!(is_error);
    let (is_error, _) = server.call("image", json!({ "code": code, "width": 100000 }));
    assert!(is_error);
}

#[test]
fn export_writes_files() {
    let mut server = Server::start();
    let dir = temp_dir("export");
    let stl = dir.join("part.stl");
    let (is_error, report) = server.report("export", json!({ "code": "cube(10);", "output": stl }));
    assert!(!is_error, "{report}");
    assert_eq!(report["written"]["format"], "STL");
    assert!(std::fs::read_to_string(&stl).unwrap().starts_with("solid Code2Form\n"));

    let package = dir.join("part.out");
    let (is_error, report) = server.report("export", json!({ "code": "cube(10);", "output": package, "format": "3mf" }));
    assert!(!is_error, "{report}");
    let bytes = std::fs::read(&package).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"));
    assert_eq!(report["written"]["bytes"], bytes.len());

    let (is_error, report) = server.report("export", json!({ "code": "square(10);", "output": dir.join("plate.svg") }));
    assert!(!is_error, "{report}");
    assert!(std::fs::read_to_string(dir.join("plate.svg")).unwrap().contains("<svg"));

    // A 2D model has no STL, and the format must be known.
    let (is_error, _) = server.call("export", json!({ "code": "square(10);", "output": dir.join("plate.stl") }));
    assert!(is_error);
    assert!(!dir.join("plate.stl").exists());
    let (is_error, _) = server.call("export", json!({ "code": "cube(10);", "output": dir.join("part.obj") }));
    assert!(is_error);
    let _ = std::fs::remove_dir_all(&dir);
}

fn base64_decode(text: &str) -> Vec<u8> {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut bits, mut count) = (0u32, 0);
    for c in text.bytes().filter(|&c| c != b'=') {
        let digit = DIGITS.iter().position(|&d| d == c).expect("a base64 digit") as u32;
        bits = (bits << 6) | digit;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    out
}

/// The size and RGB pixels of an 8-bit RGB PNG whose rows are not
/// filtered, as `c2f` writes them.
fn decode_png(png: &[u8]) -> (usize, usize, Vec<u8>) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let (mut at, mut size, mut idat) = (8, (0, 0), Vec::new());
    while at < png.len() {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let (kind, data) = (&png[at + 4..at + 8], &png[at + 8..at + 8 + length]);
        match kind {
            b"IHDR" => {
                size = (u32::from_be_bytes(data[..4].try_into().unwrap()), u32::from_be_bytes(data[4..8].try_into().unwrap()));
                assert_eq!(&data[8..10], &[8, 2], "8-bit RGB");
            }
            b"IDAT" => idat.extend_from_slice(data),
            _ => {}
        }
        at += 12 + length;
    }
    let (width, height) = (size.0 as usize, size.1 as usize);
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).expect("zlib data");
    assert_eq!(raw.len(), (width * 3 + 1) * height);
    let rgb = raw.chunks_exact(width * 3 + 1).flat_map(|row| row[1..].to_vec()).collect();
    (width, height, rgb)
}
