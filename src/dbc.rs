use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug)]
pub struct SignalDef {
    pub name: &'static str,
    pub start_bit: u16,
    pub bit_len: u8,
    pub little_endian: bool,
    pub is_signed: bool,
    pub factor: f64,
    pub offset: f64,
    pub unit: &'static str,
}

#[derive(Debug)]
pub struct MessageDef {
    pub id: u32,
    pub name: &'static str,
    pub dlc: u8,
    pub transmitter: &'static str,
    pub signals: &'static [SignalDef],
}

#[derive(Debug)]
pub struct BusDb {
    pub interface: &'static str,
    pub bus_units: &'static [&'static str],
    pub messages: &'static [MessageDef],
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/dbc_generated.rs"));
}

pub fn buses() -> &'static [BusDb] {
    generated::BUS_DBS
}

pub fn find_bus(interface: &str) -> Option<&'static BusDb> {
    buses().iter().find(|b| b.interface == interface)
}

pub fn lookup_message(interface: &str, frame_id: u32) -> Option<&'static MessageDef> {
    static LOOKUP: OnceLock<HashMap<&'static str, HashMap<u32, &'static MessageDef>>> = OnceLock::new();

    let map = LOOKUP.get_or_init(|| {
        let mut out = HashMap::new();
        for bus in buses() {
            let mut by_id = HashMap::new();
            for msg in bus.messages {
                by_id.insert(msg.id, msg);
            }
            out.insert(bus.interface, by_id);
        }
        out
    });

    map.get(interface).and_then(|by_id| by_id.get(&frame_id).copied())
}

pub fn bu_matches_message(message: &MessageDef, bu: &str, role: crate::can::BuRole) -> bool {
    let tx_match = message.transmitter == bu;

    match role {
        crate::can::BuRole::Tx => tx_match,
    }
}
