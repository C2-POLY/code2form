//! The MCP prompts: the way from an idea to a printed object with its
//! electronics, which the client offers the user and the agent follows
//! step by step. The plan comes first: questions that follow the user's
//! answers, a mechanical review, and a brief the user approves. Then the
//! model, held to the brief, and only then the electronics, each step
//! ending on what the user checks.

use serde_json::{Value, json};

/// A prompt, its arguments (name, description, required) and its text,
/// where `{name}` stands for an argument.
struct Prompt {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    arguments: &'static [(&'static str, &'static str, bool)],
    /// Arguments of which at least one is needed.
    one_of: &'static [&'static str],
    text: &'static str,
}

const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "plan_object",
        title: "Plan an object",
        description: "Before anything is drawn: asks the user what the object must do, in short rounds of questions that \
                      follow their answers, thinks it through as an engineer (masses, balance, forces and supports, \
                      torque, material, fastening, assembly, energy), and writes the brief the user approves, which \
                      `design_object` and `add_electronics` then hold to.",
        arguments: &[
            ("idea", "The idea, in a sentence or two.", true),
            ("folder", "Where to save the brief and the design (default: ~/Documents/Code2Form/<name>).", false),
        ],
        one_of: &[],
        text: "Plan this object with the user, before anything is drawn: {idea}\n\
               \n\
               Save the brief in: {folder}\n\
               \n\
               You draw nothing in this step. Understand the idea, then think it through as an engineer would, so that what works on paper also works once printed and assembled. Talk with the user in their language. The plan ends on a brief they approve.\n\
               \n\
               ## 1. The questionnaire\n\
               \n\
               Ask in small rounds of two to four questions; when your client has a tool for multiple-choice questions, use it. Give each question a sensible default the user can just accept. Ask only what changes the design: skip a block that does not apply (no electronics, no electronics questions), and let each answer open the questions it raises.\n\
               \n\
               - Use: what it does, where it lives, who handles it, how often.\n\
               - Size: its overall size, what it must hold or fit (a phone, a tube, a shelf, a watch), what it must not exceed.\n\
               - Pieces: one or several; what moves (turns, slides, opens, clips), how they go together, what must come apart.\n\
               - Making: the printer and its bed, the material, with or without supports, tolerances, colors that need separate pieces.\n\
               - Electronics, only if it has any: the components and their product pages, the power (USB, battery, adapter), what must stay reachable (a port, a button), heat.\n\
               - Look: the style, rounded or sharp, references (links, photos).\n\
               \n\
               Answers open questions. For instance:\n\
               - a motor: what it drives, that load's mass and where it sits, the speed, how quiet, continuous or occasional, whether the motor is chosen;\n\
               - something that turns or carries weight: what holds it (the motor's own shaft, a bearing, a bushing), and on which side the weight hangs;\n\
               - a battery: how long it must last;\n\
               - something that stands: on what, and whether it gets knocked;\n\
               - outdoors, near heat or in a car: the temperature (PLA softens around 55 °C).\n\
               \n\
               ## 2. The mechanical review\n\
               \n\
               Then think the object through before drawing it. Go through each step that applies and skip the others in a line: a box with a lid needs a few lines, a mechanism needs numbers.\n\
               \n\
               1. Masses: what weighs and where. The printed pieces (volume x density x the share printed; PLA 1.24 g/cm3), the components (their sheet's `mass`), what the object carries (a watch, a phone, water). Once a draft exists, measure them with `mass_properties`.\n\
               2. Balance: where the center of gravity falls over the base, and how far the object tilts before it tips (`mass_properties` gives the angle). A weight held out in front pulls the center forward: widen or weigh down the base, or bring the weight back.\n\
               3. Forces and supports: every overhang, and what carries it. A small motor's shaft and gearbox are made to turn, not to carry a load hung on them: a weight in front of the motor bends its shaft and wears its gearbox. Carry the weight on a bearing (or two) near it, and join the shaft to the motor with a coupling that passes only the rotation. Check the radial and axial loads on each support, and how much the shaft bends, with `loads`.\n\
               4. Torque and speed: the torque needed (a mass m offset by r from a horizontal axis needs m g r, plus friction), the motor's torque from its sheet, and a margin of at least 2. If it falls short: a reduction (gears, a belt) of the ratio `loads` suggests, or another motor. Then the speed after the reduction.\n\
               5. Material and printing: walls under load (at least 3 perimeters where screwed or loaded); the layer direction (a print breaks between its layers: lay it so the load runs along them); heat from a motor or LEDs; creep under a load that never stops (PETG, or a metal insert); supports and bridges.\n\
               6. Fastening and vibration: screws into heat-set inserts or captive nuts rather than into plastic; what loosens when it turns or shakes (a set screw on a flat, thread locker); what must come apart for upkeep.\n\
               7. Assembly: the order of assembly, room for fingers and the screwdriver, access to the screws and ports once closed, how the cables are held and relieved.\n\
               8. Energy: the current each part draws, the supply's budget with a margin, a battery's life.\n\
               \n\
               For each risk, write the assumption and the calculation with its orders of magnitude; the decision you propose (a bearing, a wider base, a reduction, another motor, an insert); and what stays uncertain and how to check it (weigh the watch, a test print, a test run). Find the bearings, screws, inserts, gears and motors you propose with `library_parts`, with their figures. Whenever a tool can make a calculation, prefer it to an estimate, and say which numbers are estimates.\n\
               \n\
               Show the user the review in short, with the decisions it leads to, and ask them to agree or change them.\n\
               \n\
               ## 3. The brief\n\
               \n\
               Write the brief with `save_brief`, as `brief.md` in the folder: the title; the idea and its use; sizes and constraints; the pieces and how they go together; the components with their links; the review (risks, calculations, decisions, what is left to verify); and the `## Requirements` that hold the design to those decisions, each a line a tool measures: `mass < ... g`, `tipping > ...°`, `torque margin >= 2`, `radial load margin >= 2`, `size <= X x Y x Z mm` (the printer), `part \"Name\"` for each planned piece, `component \"electronics/....c2f\"` for each component. Put what the tools compute from in a ```json mass``` block (material, infill, the extra masses carried, as `mass_properties` takes them) and a ```json loads``` block (the shaft, its loads, its supports and the motor, as `loads` takes them).\n\
               \n\
               Show the user the brief and ask them to approve it. Then `design_object` with this `brief` takes it from there, and later `add_electronics`.",
    },
    Prompt {
        name: "design_object",
        title: "Design an object",
        description: "Turns a brief or a description into a parametric Code2Form design, shown from every side, \
                      checked against the brief, and adjusted until the user approves it. The step before any electronics.",
        arguments: &[
            ("description", "What the object is and does, its size, how it is used.", false),
            ("brief", "The brief `plan_object` wrote (its .md file).", false),
            ("constraints", "How it is made: printer bed, material, wall thickness, tolerances.", false),
            ("folder", "Where to save the design (default: the brief's folder, or ~/Documents/Code2Form/<name>).", false),
        ],
        one_of: &["description", "brief"],
        text: "Design this object with Code2Form, as a parametric `.c2f` design:\n\
               \n\
               {description}\n\
               \n\
               Brief: {brief}\n\
               Constraints: {constraints}\n\
               Save it in: {folder}\n\
               \n\
               Work in this order, and stop where it says to ask:\n\
               \n\
               1. If there is a brief, read it with `read_brief` first: its decisions (a bearing, a reduction, a wider base) and its requirements are part of the design, and the description only adds to them. Restate what you understood in a few lines: the parts, their sizes, how they fit, what moves. Ask about anything that changes the shape before modelling.\n\
               2. Write the design: its main sizes as customizer parameters (`width = 40; // [20:80]`, grouped with `/* [Group] */`), each printed piece a `part(\"name\")`, comments that say why. Bought parts (motors, boards, bearings) are drawn in grey and stay out of the printed pieces; seat a bearing with the library's `bearing_seat`, mesh gears with `gear_pair`.\n\
               3. Render it with `render`; fix every error and warning.\n\
               4. Look at it with `image` and `view = \"sheet\"` (add `section = true` for a hollow piece), and check it against the description: sizes, proportions, nothing floating, walls thick enough to print. When it carries weight or turns, measure it with `mass_properties` and `loads`.\n\
               5. If there is a brief, check the design against it with `check_brief`: fix what does not hold, or tell the user why it cannot.\n\
               6. Show the user the sheet, list the parameters and what you assumed, and ask them to approve it or say what to change. Adjust and show it again until they approve.\n\
               \n\
               Do not add electronics yet: once the user approves the design, the `add_electronics` prompt takes it from there.",
    },
    Prompt {
        name: "add_electronics",
        title: "Add the electronics",
        description: "Turns an approved design into an electronics build: the parts created from their \
                      product pages, placed in the design and wired, the design given their standoffs, \
                      cutouts and wire channels, and everything checked, against the brief too.",
        arguments: &[
            ("design", "The approved design's `.c2f` file.", true),
            ("components", "The components: product page links, or names of parts in the library.", true),
            ("behaviour", "What the electronics does: inputs, outputs, power.", false),
            ("brief", "The brief `plan_object` wrote (its .md file).", false),
        ],
        one_of: &[],
        text: "Add the electronics to the approved design {design}.\n\
               \n\
               Components: {components}\n\
               What it does: {behaviour}\n\
               Brief: {brief}\n\
               \n\
               Work in this order, and stop where it says to ask:\n\
               \n\
               1. If there is a brief, read it with `read_brief`: its components, decisions and requirements come first. Find each component with `library_parts` and a `query` describing it. For one the library lacks, read its product page and datasheet, write the part (its board or body to scale, `// @pin` with type and voltage, `// @mount_hole`, `// @port`, and its `// @description`, `// @spec` (with its `mass`, and for a motor its `torque` and `speed`) and `// @source`), check it with `check_part`, look at it with `image`, and save it with `save_part`.\n\
               2. Plan the wiring: power, ground, and each signal on a pin that can take it (no strapping or serial pins without a reason). Tell the user the plan and the parts list before going on.\n\
               3. Call `electrify` on the design: it creates the circuit file beside it and keeps the design as it is, with empty `electronics_cuts()` and `electronics_mounts()` applied to it (for a design made of pieces, to the `piece` that holds the electronics).\n\
               4. Place the parts in the circuit (`at`, `rotate`, in the design's coordinates) where they fit and can be reached (ports at a wall, the board on standoffs), and write the wires, power first. Check with `check_circuit` and `fit`; fix every error, and every warning you cannot explain.\n\
               5. Give the design what the parts need, in `electronics_cuts()` (`cutout_for`, `holes_for`, `channel_for`) and `electronics_mounts()` (`standoffs_for`), render it, and check again with `fit`.\n\
               6. If there is a brief, check the build against it with `check_brief` (the circuit gives the components' masses): fix what does not hold, or tell the user why it cannot.\n\
               7. Show the user the build with `circuit_image` (`view = \"sheet\"` when it helps), the wiring list from `circuit_wires` and the parts from `circuit_bom`, and ask them to approve it or say what to change.\n\
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
    let has = |argument: &str| given.get(argument).and_then(Value::as_str).is_some_and(|v| !v.trim().is_empty());
    if !prompt.one_of.is_empty() && !prompt.one_of.iter().any(|a| has(a)) {
        let names: Vec<String> = prompt.one_of.iter().map(|a| format!("`{a}`")).collect();
        return Err((-32602, format!("the prompt `{name}` needs {}", names.join(" or "))));
    }
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

    #[test]
    fn the_plan_asks_in_blocks_then_reviews_the_mechanics() {
        let plan = &PROMPTS[0];
        assert_eq!(plan.name, "plan_object");
        for block in ["- Use:", "- Size:", "- Pieces:", "- Making:", "- Electronics, only if it has any:", "- Look:"] {
            assert!(plan.text.contains(block), "{block}");
        }
        assert!(plan.text.contains("skip a block that does not apply") && plan.text.contains("Answers open questions"));
        for step in ["Masses:", "Balance:", "Forces and supports:", "Torque and speed:", "Material and printing:", "Fastening and vibration:", "Assembly:", "Energy:"] {
            assert!(plan.text.contains(step), "{step}");
        }
        assert!(plan.text.contains("`save_brief`") && plan.text.contains("## Requirements"));
    }

    #[test]
    fn a_design_starts_from_a_description_or_a_brief() {
        assert!(get(&json!({ "name": "design_object", "arguments": {} })).is_err());
        let got = get(&json!({ "name": "design_object", "arguments": { "brief": "winder/brief.md" } })).unwrap();
        let text = got["messages"][0]["content"]["text"].as_str().unwrap();
        assert!(text.contains("Brief: winder/brief.md") && text.contains("`check_brief`"), "{text}");
    }
}
