//! The serial ports a board can be on, the boards Code2Form knows first.

use serialport::{SerialPortInfo, SerialPortType};

/// A serial port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    /// What to open: `/dev/cu.usbmodem1101`, `COM3`.
    pub name: String,
    /// The board or the USB chip its USB ids tell, when known.
    pub board: Option<&'static str>,
    /// What the port says it is: its product name, or its USB ids.
    pub description: String,
    /// Whether it is a USB port: a board plugged in, rather than a port of
    /// the computer.
    pub usb: bool,
}

/// The boards and USB serial chips, by USB vendor and product id (`None`
/// for any product of the vendor).
const KNOWN: [(u16, Option<u16>, &str); 7] = [
    // The ESP32-S3's own USB, as on the XIAO ESP32-S3 and the DevKitC-1's
    // USB port.
    (0x303A, Some(0x1001), "ESP32-S3"),
    (0x303A, None, "Espressif"),
    (0x2886, None, "Seeed Studio"),
    // The USB serial chips on the DevKits' UART port.
    (0x10C4, Some(0xEA60), "CP210x"),
    (0x1A86, Some(0x7523), "CH340"),
    (0x1A86, Some(0x55D4), "CH9102"),
    (0x0403, Some(0x6001), "FTDI"),
];

/// The board behind a USB serial port, from its ids.
pub fn known(vid: u16, pid: u16) -> Option<&'static str> {
    KNOWN.iter().find(|(v, p, _)| *v == vid && p.is_none_or(|p| p == pid)).map(|(_, _, name)| *name)
}

/// The serial ports, the known boards first, then the other USB ports.
/// On macOS each port shows once, as its `cu.` device, and the Bluetooth
/// and debug ports are left out.
pub fn list() -> Vec<Port> {
    let mut ports: Vec<Port> = serialport::available_ports().unwrap_or_default().into_iter().filter_map(port).collect();
    ports.sort_by_key(|p| (p.board.is_none(), !p.usb, p.name.clone()));
    ports.dedup_by(|a, b| a.name == b.name);
    ports
}

/// The port to use: the first known board's.
pub fn preferred(ports: &[Port]) -> Option<&Port> {
    ports.iter().find(|p| p.board.is_some())
}

fn port(info: SerialPortInfo) -> Option<Port> {
    let name = info.port_name;
    if cfg!(target_os = "macos") && (name.starts_with("/dev/tty.") || name.contains("Bluetooth") || name.contains("debug-console")) {
        return None;
    }
    let (board, description, usb) = match info.port_type {
        SerialPortType::UsbPort(usb) => (
            known(usb.vid, usb.pid),
            usb.product.unwrap_or_else(|| format!("USB {:04x}:{:04x}", usb.vid, usb.pid)),
            true,
        ),
        SerialPortType::BluetoothPort => return None,
        SerialPortType::PciPort | SerialPortType::Unknown => (None, String::new(), false),
    };
    Some(Port { name, board, description, usb })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boards_are_told_by_their_usb_ids() {
        assert_eq!(known(0x303A, 0x1001), Some("ESP32-S3"));
        assert_eq!(known(0x303A, 0x4001), Some("Espressif"));
        assert_eq!(known(0x10C4, 0xEA60), Some("CP210x"));
        assert_eq!(known(0x05AC, 0x1234), None);
    }

    #[test]
    fn a_known_board_is_preferred() {
        let ports = [
            Port { name: "/dev/cu.other".into(), board: None, description: String::new(), usb: false },
            Port { name: "/dev/cu.usbmodem1101".into(), board: Some("ESP32-S3"), description: "USB JTAG/serial debug unit".into(), usb: true },
        ];
        assert_eq!(preferred(&ports).map(|p| p.name.as_str()), Some("/dev/cu.usbmodem1101"));
        assert_eq!(preferred(&ports[..1]), None);
    }
}
