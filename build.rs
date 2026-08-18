use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct Dbc {
    bus_units: Vec<String>,
    messages: Vec<Message>,
}

#[derive(Debug, Default)]
struct Message {
    id: u32,
    name: String,
    dlc: u8,
    transmitter: String,
    signals: Vec<Signal>,
}

#[derive(Debug, Default)]
struct Signal {
    name: String,
    start_bit: u16,
    bit_len: u8,
    little_endian: bool,
    is_signed: bool,
    factor: f64,
    offset: f64,
    unit: String,
}

fn escape_rust_str(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

fn format_f64_literal(value: f64) -> String {
    let s = value.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{}.0", s)
    }
}

fn parse_dbc(path: &Path) -> Result<Dbc> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => {
            println!(
                "cargo:warning=DBC file contains invalid UTF-8; using lossy decoding: {}",
                path.display()
            );
            String::from_utf8_lossy(&err.into_bytes()).into_owned()
        }
    };

    let msg_re = Regex::new(r"^BO_\s+(\d+)\s+([^:]+):\s+(\d+)\s+(\S+)\s*$")?;
    let sig_re = Regex::new(
        r#"^SG_\s+([^\s:]+)(?:\s+\S+)?\s*:\s*(\d+)\|(\d+)@([01])([+-])\s+\(([-+0-9.eE]+),([-+0-9.eE]+)\)\s+\[[^\]]*\]\s+"([^"]*)"\s*(.*)$"#,
    )?;

    let mut dbc = Dbc::default();
    let mut current_message: Option<Message> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("BU_:") {
            dbc.bus_units = rest
                .split_whitespace()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            continue;
        }

        if let Some(caps) = msg_re.captures(trimmed) {
            if let Some(msg) = current_message.take() {
                dbc.messages.push(msg);
            }

            current_message = Some(Message {
                id: caps[1].parse().context("invalid message id")?,
                name: caps[2].trim().to_string(),
                dlc: caps[3].parse().context("invalid message dlc")?,
                transmitter: caps[4].trim().to_string(),
                signals: Vec::new(),
            });
            continue;
        }

        if let Some(caps) = sig_re.captures(trimmed) {
            if let Some(ref mut msg) = current_message {
                msg.signals.push(Signal {
                    name: caps[1].trim().to_string(),
                    start_bit: caps[2].parse().context("invalid signal start bit")?,
                    bit_len: caps[3].parse().context("invalid signal length")?,
                    little_endian: &caps[4] == "1",
                    is_signed: &caps[5] == "-",
                    factor: caps[6].parse().context("invalid signal factor")?,
                    offset: caps[7].parse().context("invalid signal offset")?,
                    unit: caps[8].to_string(),
                });
            }
            continue;
        }
    }

    if let Some(msg) = current_message.take() {
        dbc.messages.push(msg);
    }

    Ok(dbc)
}

fn emit_dbc_module(interface_name: &str, dbc: &Dbc, out: &mut String) {
    let module_name = interface_name.replace('-', "_");
    out.push_str(&format!("pub mod {} {{\n", module_name));
    out.push_str("    use super::super::{MessageDef, SignalDef};\n");

    for (msg_idx, msg) in dbc.messages.iter().enumerate() {
        out.push_str(&format!(
            "    static MSG_{}_SIGNALS: &[SignalDef] = &[\n",
            msg_idx
        ));
        for sig in &msg.signals {
            out.push_str(&format!(
                "        SignalDef {{ name: \"{}\", start_bit: {}, bit_len: {}, little_endian: {}, is_signed: {}, factor: {}, offset: {}, unit: \"{}\" }},\n",
                escape_rust_str(&sig.name),
                sig.start_bit,
                sig.bit_len,
                sig.little_endian,
                sig.is_signed,
                format_f64_literal(sig.factor),
                format_f64_literal(sig.offset),
                escape_rust_str(&sig.unit)
            ));
        }
        out.push_str("    ];\n");
    }

    out.push_str("    pub static BUS_UNITS: &[&str] = &[\n");
    for bu in &dbc.bus_units {
        out.push_str(&format!("        \"{}\",\n", escape_rust_str(bu)));
    }
    out.push_str("    ];\n");

    out.push_str("    pub static MESSAGES: &[MessageDef] = &[\n");
    for (msg_idx, msg) in dbc.messages.iter().enumerate() {
        out.push_str(&format!(
            "        MessageDef {{ id: {}, name: \"{}\", dlc: {}, transmitter: \"{}\", signals: MSG_{}_SIGNALS }},\n",
            msg.id,
            escape_rust_str(&msg.name),
            msg.dlc,
            escape_rust_str(&msg.transmitter),
            msg_idx
        ));
    }
    out.push_str("    ];\n");

    out.push_str("}\n\n");
}

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=dbc/P952_E3A_R1_BHCAN.dbc");
    println!("cargo:rerun-if-changed=dbc/P952_E3A_R1_C1CAN.dbc");
    println!("cargo:rerun-if-changed=dbc/P952_E3A_R1_C2CAN.dbc");

    let dbcs = [
        ("can-bh", PathBuf::from("dbc/P952_E3A_R1_BHCAN.dbc")),
        ("can-c1", PathBuf::from("dbc/P952_E3A_R1_C1CAN.dbc")),
        ("can-c2", PathBuf::from("dbc/P952_E3A_R1_C2CAN.dbc")),
    ];

    let mut out = String::new();
    out.push_str("// @generated by build.rs\n");
    out.push_str("use super::BusDb;\n\n");

    for (iface, path) in &dbcs {
        let dbc = parse_dbc(path)?;
        emit_dbc_module(iface, &dbc, &mut out);
    }

    out.push_str("pub static BUS_DBS: &[BusDb] = &[\n");
    for (iface, _) in &dbcs {
        let module_name = iface.replace('-', "_");
        out.push_str(&format!(
            "    BusDb {{ interface: \"{}\", bus_units: {}::BUS_UNITS, messages: {}::MESSAGES }},\n",
            iface, module_name, module_name
        ));
    }
    out.push_str("];\n");

    let out_dir = std::env::var("OUT_DIR").context("OUT_DIR not set")?;
    let output_path = Path::new(&out_dir).join("dbc_generated.rs");
    fs::write(output_path, out).context("failed writing generated dbc module")?;

    Ok(())
}
