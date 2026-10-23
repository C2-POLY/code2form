//! The MCP prompts: the way from an idea to a printed object with its
//! electronics, which the client offers the user and the agent follows
//! step by step. The model comes first and the user approves it; only then
//! does the electronics go in, each step ending on what the user checks.

use serde_json::{Value, json};

/// A prompt, its arguments (name, description, required) and its text,
/// where `{name}` stands for an argument.
struct Prompt {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    arguments: &'static [(&'static str, &'static str, bool)],
    text: &'static str,
}

const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "design_object",
        title: "Design an object",
        description: "Turns a description into a parametric Code2Form design, shown from every side, \
                      and adjusted until the user approves it. The first step before any electronics.",
        arguments: &[
            ("description", "What the object is and does, its size, how it is used.", true),
            ("constraints", "How it is made: printer bed, material, wall thickness, tolerances.", false),
            ("folder", "Where to save the design (default: ~/Documents/Code2Form/<name>).", false),
        ],
        text: "Design this object with Code2Form, as a parametric `.c2f` design:\n\
               \n\
               {description}\n\
               \n\
               Constraints: {constraints}\n\
               Save it in: {folder}\n\
               \n\
               Work in this order, and stop where it says to ask:\n\
               \n\
               1. Restate what you understood in a few lines: the parts, their sizes, how they fit, \
               what moves. Ask about anything that changes the shape before modelling.\n\
               2. Write the design: its main sizes as customizer parameters (`width = 40; // [20:80]`, \
               grouped with `/* [Group] */`), each printed piece a `part(\"name\")`, comments that say why. \
               Bought parts (motors, boards, bearings) are drawn in grey and stay out of the printed pieces.\n\
               3. Render it with `render`; fix every error and warning.\n\
               4. Look at it with `image` and `view = \"sheet\"` (add `section = true` for a hollow \
               piece), and check it against the description: sizes, proportions, nothing floating, \
               walls thick enough to print.\n\
               5. Show the user the sheet, list the parameters and what you assumed, and ask them to \
               approve it or say what to change. Adjust and show it again until they approve.\n\
               \n\
               Do not add electronics yet: once the user approves the design, the `add_electronics` \
               prompt takes it from there.",
    },
    Prompt {
        name: "add_electronics",
        title: "Add the electronics",
        description: "Turns an approved design into an electronics build: the parts created from their \
                      product pages, placed in the design and wired, the design given their standoffs, \
                      cutouts and wire channels, and everything checked.",
        arguments: &[
            ("design", "The approved design's `.c2f` file.", true),
            ("components", "The components: product page links, or names of parts in the library.", true),
            ("behaviour", "What the electronics does: inputs, outputs, power.", false),
        ],
        text: "Add the electronics to the approved design {design}.\n\
               \n\
               Components: {components}\n\
               What it does: {behaviour}\n\
               \n\
               Work in this order, and stop where it says to ask:\n\
               \n\
               1. Find each component with `library_parts` and a `query` describing it. For one the \
               library lacks, read its product page and datasheet, write the part (its board or body \
               to scale, `// @pin` with type and voltage, `// @mount_hole`, `// @port`, and its \
               `// @description`, `// @spec` and `// @source`), check it with `check_part`, look at it \
               with `image`, and save it with `save_part`.\n\
               2. Plan the wiring: power, ground, and each signal on a pin that can take it (no \
               strapping or serial pins without a reason). Tell the user the plan and the parts list \
               before going on.\n\
               3. Call `electrify` on the design: it creates the circuit file beside it and keeps the \
               design as it is, in a `design()` module, with empty `electronics_cuts()` and \
               `electronics_mounts()`.\n\
               4. Place the parts in the circuit (`at`, `rotate`, in the design's coordinates) where \
               they fit and can be reached (ports at a wall, the board on standoffs), and write the \
               wires, power first. Check with `check_circuit` and `fit`; fix every error, and every \
               warning you cannot explain.\n\
               5. Give the design what the parts need, in `electronics_cuts()` (`cutout_for`, \
               `holes_for`, `channel_for`) and `electronics_mounts()` (`standoffs_for`), render it, and \
               check again with `fit`.\n\
               6. Show the user the build with `circuit_image` (`view = \"sheet\"` when it helps), the \
               wiring list from `circuit_wires` and the parts from `circuit_bom`, and ask them to \
               approve it or say what to change.\n\
               \n\
               Once approved, `export` writes each printed piece for the slicer.",
    },
];

/// `prompts/list`.
pub fn list() -> Value {
    let prompts: Vec<Value> = PROMPTS
        .iter()
        .map(|p| {
            let arguments: Vec<Value> = p
                .arguments
                .iter()
                .map(|(name, description, required)| json!({ "name": name, "description": description, "required": required }))
                .collect();
            json!({ "name": p.name, "title": p.title, "description": p.description, "arguments": arguments })
        })
        .collect();
    json!({ "prompts": prompts })
}

/// `prompts/get`: the prompt's text with its arguments filled in; an
/// optional one left out reads "not given".
pub fn get(params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "`name` names the prompt".to_string()))?;
    let prompt = PROMPTS.iter().find(|p| p.name == name).ok_or((-32602, format!("unknown prompt: {name}")))?;
    let given = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let mut text = prompt.text.to_string();
    for (argument, _, required) in prompt.arguments {
        let value = given.get(*argument).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        let value = match value {
            Some(value) => value.to_string(),
            None if *required => return Err((-32602, format!("the prompt `{name}` needs `{argument}`"))),
            None => "not given".to_string(),
        };
        text = text.replace(&format!("{{{argument}}}"), &value);
    }
    Ok(json!({
        "description": prompt.description,
        "messages": [{ "role": "user", "content": { "type": "text", "text": text } }],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_argument_is_used_by_its_text() {
        for prompt in PROMPTS {
            for (argument, _, _) in prompt.arguments {
                assert!(prompt.text.contains(&format!("{{{argument}}}")), "{}: {argument}", prompt.name);
            }
        }
    }

    #[test]
    fn a_prompt_fills_its_arguments() {
        let got = get(&json!({ "name": "design_object", "arguments": { "description": "A watch winder" } })).unwrap();
        let text = got["messages"][0]["content"]["text"].as_str().unwrap();
        assert!(text.contains("A watch winder") && text.contains("Constraints: not given"), "{text}");
        assert!(get(&json!({ "name": "add_electronics", "arguments": { "design": "a.c2f" } })).is_err());
        assert!(get(&json!({ "name": "nothing" })).is_err());
    }
}
