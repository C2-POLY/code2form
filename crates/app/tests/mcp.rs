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
        Server::start_with(&[])
    }

    /// The server with these environment variables set.
    fn start_with(env: &[(&str, &std::ffi::OsStr)]) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_c2f"))
            .arg("mcp")
            .envs(env.iter().copied())
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
    assert_eq!(
        names,
        ["render", "parameters", "image", "export", "library_parts", "part_pins", "check_circuit", "circuit_image", "circuit_wires", "circuit_bom"]
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        let takes_code = tool["inputSchema"]["properties"]["code"].is_object();
        assert_eq!(takes_code, tool["name"] != "library_parts", "{tool}");
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
fn parameters_set_the_time() {
    let mut server = Server::start();
    let (is_error, report) = server.report("render", json!({ "code": "echo($t);", "parameters": { "$t": 0.25 } }));
    assert!(!is_error, "{report}");
    assert_eq!(report["echo"], json!(["ECHO: 0.25"]));
    let (is_error, _) = server.call("render", json!({ "code": "cube(1);", "parameters": { "$t": "soon" } }));
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

    let gltf = dir.join("part.glb");
    let (is_error, report) =
        server.report("export", json!({ "code": "color(\"red\") cube(10);", "output": gltf, "scale": 1 }));
    assert!(!is_error, "{report}");
    assert_eq!((report["written"]["format"].clone(), report["triangles"].clone()), (json!("glTF"), json!(12)));
    let bytes = std::fs::read(&gltf).unwrap();
    assert!(bytes.starts_with(b"glTF"));
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let document: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    assert_eq!(document["nodes"][0]["name"], "part");
    assert_eq!(document["accessors"][0]["max"], json!([10.0, 10.0, 0.0]));
    let (is_error, _) = server.call("export", json!({ "code": "cube(1);", "output": gltf, "scale": 0 }));
    assert!(is_error);
    let code = "rotate([0, 0, 360 * $t]) part(\"wheel\") cube(5);";
    let (is_error, report) = server.report("export", json!({ "code": code, "output": gltf, "fps": 10, "loop": false }));
    assert!(!is_error, "{report}");
    assert_eq!(report["animation"], json!({ "name": "part", "frames": 21 }));
    let (is_error, report) = server.report("export", json!({ "code": code, "output": gltf, "animate": false }));
    assert!(!is_error && report.get("animation").is_none(), "{report}");

    let (is_error, report) = server.report("export", json!({ "code": "square(10);", "output": dir.join("plate.svg") }));
    assert!(!is_error, "{report}");
    assert!(std::fs::read_to_string(dir.join("plate.svg")).unwrap().contains("<svg"));

    // A 2D model has no STL, and the format must be known.
    let (is_error, _) = server.call("export", json!({ "code": "square(10);", "output": dir.join("plate.stl") }));
    assert!(is_error);
    assert!(!dir.join("plate.stl").exists());
    let (is_error, _) = server.call("export", json!({ "code": "cube(10);", "output": dir.join("part.xyz") }));
    assert!(is_error);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_writes_files_for_other_programs() {
    let mut server = Server::start();
    let dir = temp_dir("exchange");
    let arm = "cube(10); translate([0, 0, 10]) part(\"elbow\", joint = \"revolute\", limits = [-90, 90]) color(\"red\") cube([2, 2, 20]);";

    let (is_error, report) = server.report("export", json!({ "code": arm, "output": dir.join("arm.obj") }));
    assert!(!is_error, "{report}");
    assert_eq!(report["written"]["format"], "OBJ");
    assert!(std::fs::read_to_string(dir.join("arm.obj")).unwrap().contains("\no elbow\n"));
    assert!(std::fs::read_to_string(dir.join("arm.mtl")).unwrap().contains("Kd 1.0 0.0 0.0"));

    let (is_error, report) = server.report("export", json!({ "code": arm, "output": dir.join("arm.step") }));
    assert!(!is_error, "{report}");
    assert!(std::fs::read_to_string(dir.join("arm.step")).unwrap().starts_with("ISO-10303-21;"));

    let (is_error, report) = server.report("export", json!({ "code": arm, "output": dir.join("arm.urdf"), "density": 1000 }));
    assert!(!is_error, "{report}");
    assert_eq!((report["links"].clone(), report["moving_joints"].clone()), (json!(2), json!(1)));
    assert!(std::fs::read_to_string(dir.join("arm.urdf")).unwrap().contains("type=\"revolute\""));
    assert!(dir.join("arm_meshes/elbow.stl").exists());

    let (is_error, report) = server.report("export", json!({ "code": arm, "output": dir.join("arm.json") }));
    assert!(!is_error, "{report}");
    let parts: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("arm.json")).unwrap()).unwrap();
    assert_eq!(parts["parts"][1]["joint"]["type"], "revolute");

    let (is_error, report) = server.report("export", json!({ "code": "cube([10, 20, 30]);", "output": dir.join("side.dxf"), "view": "front" }));
    assert!(!is_error, "{report}");
    assert!(std::fs::read_to_string(dir.join("side.dxf")).unwrap().contains("LINE"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn grogu_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/grogu")
}

/// The value of each `key = "value"` line of a TOML text, with the table
/// it is in.
fn toml_strings<'a>(src: &'a str, key: &str) -> Vec<(&'a str, &'a str)> {
    let mut table = "";
    let mut values = Vec::new();
    for line in src.lines().map(str::trim) {
        if line.starts_with('[') {
            table = line;
        } else if let Some(value) = line.strip_prefix(key).and_then(|rest| rest.trim_start().strip_prefix('=')) {
            values.push((table, value.trim().trim_matches('"')));
        }
    }
    values
}

#[test]
fn an_agent_builds_the_grogu_circuit() {
    // A library of one part, as the agent finds it.
    let library = temp_dir("library");
    std::fs::create_dir_all(library.join("electronics/sensors")).unwrap();
    std::fs::write(library.join("electronics/sensors/probe.c2f"), "// A probe, simplified.\n// @pin S [0, 0, 1] input 3.6V\ncube(1);\n").unwrap();
    let mut server = Server::start_with(&[("C2F_LIBRARY_PATH", library.as_os_str())]);
    let (is_error, report) = server.report("library_parts", json!({}));
    assert!(!is_error, "{report}");
    let probe = report["parts"].as_array().unwrap().iter().find(|p| p["part"] == "electronics/sensors/probe.c2f");
    assert_eq!(probe.expect("the probe is listed")["description"], "A probe, simplified.", "{report}");
    let (_, report) = server.report("part_pins", json!({ "path": "electronics/sensors/probe.c2f" }));
    assert_eq!(report["pins"][0]["voltage"], 3.6, "{report}");
    let _ = std::fs::remove_dir_all(&library);

    // The pins of each part the circuit places, then the wires between them.
    let dir = grogu_dir();
    let src = std::fs::read_to_string(dir.join("grogu.c2fc")).unwrap();
    let mut pins = std::collections::HashMap::new();
    for (table, part) in toml_strings(&src, "part") {
        let (is_error, report) = server.report("part_pins", json!({ "path": part, "working_dir": dir }));
        assert!(!is_error && report["errors"] == json!([]), "{part}: {report}");
        let names: Vec<String> = report["pins"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().into()).collect();
        assert!(report["pins"].as_array().unwrap().iter().all(|p| p["type"].is_string() && p["position"].is_array()));
        let instance = table.trim_matches(['[', ']']).strip_prefix("parts.").expect("a part's table");
        pins.insert(instance.to_string(), names);
    }
    assert_eq!(pins.len(), 8);
    for (_, end) in toml_strings(&src, "from").into_iter().chain(toml_strings(&src, "to")) {
        let (part, pin) = end.split_once('.').unwrap();
        assert!(pins[part].iter().any(|p| p == pin), "{end}");
    }

    // The circuit's text validates: no errors, the strapping pin's warning.
    let (is_error, report) = server.report("check_circuit", json!({ "code": src, "working_dir": dir }));
    assert!(!is_error, "{report}");
    assert_eq!((&report["ok"], &report["errors"], &report["parts"], &report["wires"]), (&json!(true), &json!([]), &json!(8), &json!(23)));
    assert!(report["warnings"].as_array().unwrap().iter().all(|w| w["message"].as_str().unwrap().contains("reserved")), "{report}");

    // A mistake is an error on its line.
    let wrong = src.replace("from = \"xiao.3V3\"", "from = \"xiao.5V\"");
    let (is_error, report) = server.report("check_circuit", json!({ "code": wrong, "working_dir": dir }));
    assert!(is_error);
    let error = &report["errors"][0];
    assert!(error["message"].as_str().unwrap().contains("supplies 5 V to `mic.VDD`"), "{report}");
    assert_eq!(error["line"].as_u64().unwrap() as usize, src[..src.find("from = \"xiao.3V3\"").unwrap()].lines().count() + 1);
    let (is_error, report) = server.report("check_circuit", json!({ "code": "[parts.a]\npart = \"nope.c2f\"\n", "working_dir": dir }));
    assert!(is_error);
    assert_eq!(report["errors"][0]["line"], 2, "{report}");

    // The wiring list and the bill of materials.
    let path = dir.join("grogu.c2fc");
    let (_, report) = server.report("circuit_wires", json!({ "path": path }));
    let first = &report["wires"][0];
    assert_eq!((&first["step"], &first["from"], &first["to"], &first["color"]), (&json!(1), &json!("xiao.5V"), &json!("amp.VIN"), &json!("red")));
    assert!(first["length_mm"].as_f64().unwrap() > 10.0, "{first}");
    assert_eq!(report["wires"].as_array().unwrap().len(), 23);
    let (_, report) = server.report("circuit_bom", json!({ "path": path }));
    assert_eq!(report["parts"].as_array().unwrap().len(), 8);
    assert!(report["parts"].as_array().unwrap().iter().any(|p| p["params"] == "headers=true" && p["instances"] == "amp"), "{report}");
}

#[test]
fn circuit_image_is_a_png_with_its_wires() {
    let mut server = Server::start();
    let path = grogu_dir().join("grogu.c2fc");
    let picture = |server: &mut Server, args: Value| {
        let (is_error, content) = server.call("circuit_image", args);
        assert!(!is_error, "{content:?}");
        assert_eq!(content[0]["mimeType"], "image/png");
        let report: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
        (decode_png(&base64_decode(content[0]["data"].as_str().unwrap())), report)
    };
    let ((width, height, plain), report) = picture(&mut server, json!({ "path": path, "view": "top", "width": 160, "height": 120 }));
    assert_eq!((width, height, &report["wires"]), (160, 120, &json!(23)));
    // The ground net lit, the other wires greyed: another picture.
    let ((_, _, lit), report) = picture(&mut server, json!({ "path": path, "view": "top", "width": 160, "height": 120, "net": "amp.GND" }));
    let net: Vec<&str> = report["net"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
    assert!(net.contains(&"xiao.GND") && net.contains(&"mic.L/R") && !net.contains(&"xiao.5V"), "{net:?}");
    assert_ne!(plain, lit);
    // Wires are drawn: the picture differs from the parts alone.
    let src = std::fs::read_to_string(&path).unwrap();
    let parts_only = &src[..src.find("[[wires]]").unwrap()];
    let ((_, _, bare), _) = picture(&mut server, json!({ "code": parts_only, "working_dir": grogu_dir(), "view": "top", "width": 160, "height": 120 }));
    assert_ne!(plain, bare);

    let (is_error, _) = server.call("circuit_image", json!({ "path": path, "net": "amp.NOPE" }));
    assert!(is_error);
    let (is_error, _) = server.call("circuit_image", json!({ "path": path, "view": "under" }));
    assert!(is_error);
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
