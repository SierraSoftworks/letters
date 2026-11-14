//! Which addresses a fetch may connect to.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// IPv4 ranges that are not the public internet, as a network and its prefix length.
const NOT_PUBLIC_V4: &[([u8; 4], u32)] = &[
    ([0, 0, 0, 0], 8),       // this network
    ([10, 0, 0, 0], 8),      // private
    ([100, 64, 0, 0], 10),   // shared by carriers and overlay networks
    ([127, 0, 0, 0], 8),     // this machine
    ([169, 254, 0, 0], 16),  // link-local, where cloud metadata lives
    ([172, 16, 0, 0], 12),   // private
    ([192, 0, 0, 0], 24),    // protocol assignments
    ([192, 0, 2, 0], 24),    // documentation
    ([192, 88, 99, 0], 24),  // 6to4 relays
    ([192, 168, 0, 0], 16),  // private
    ([198, 18, 0, 0], 15),   // benchmarking
    ([198, 51, 100, 0], 24), // documentation
    ([203, 0, 113, 0], 24),  // documentation
    ([224, 0, 0, 0], 3),     // multicast, reserved and broadcast
];

/// IPv6 ranges inside global unicast (`2000::/3`) that are not the public internet.
const NOT_PUBLIC_V6: &[([u16; 2], u32)] = &[
    ([0x2001, 0x0000], 23), // protocol assignments, Teredo among them
    ([0x2001, 0x0db8], 32), // documentation
    ([0x3fff, 0x0000], 20), // documentation
];

/// Whether an address is on the public internet. A fetch connects to nothing else, so a document cannot
/// reach this machine, the local network or a cloud host's metadata service.
///
/// Check every address a name resolves to, and connect to the address that was checked: a second lookup may
/// answer differently.
pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => public_v4(address),
        IpAddr::V6(address) => match embedded_v4(address) {
            Some(inner) => public_v4(inner),
            None => public_v6(address),
        },
    }
}

fn public_v4(address: Ipv4Addr) -> bool {
    let bits = u32::from(address);
    !NOT_PUBLIC_V4
        .iter()
        .any(|&(network, prefix)| bits >> (32 - prefix) == u32::from(Ipv4Addr::from(network)) >> (32 - prefix))
}

fn public_v6(address: Ipv6Addr) -> bool {
    let bits = u128::from(address);
    let within = |head: [u16; 2], prefix: u32| {
        let network = (u128::from(head[0]) << 112) | (u128::from(head[1]) << 96);
        bits >> (128 - prefix) == network >> (128 - prefix)
    };
    // Everything outside global unicast is local, multicast or reserved.
    within([0x2000, 0], 3) && !NOT_PUBLIC_V6.iter().any(|&(head, prefix)| within(head, prefix))
}

/// The IPv4 address an IPv6 address stands for: mapped (`::ffff:a.b.c.d`), translated (`64:ff9b::a.b.c.d`)
/// or tunnelled (`2002:aabb:ccdd::`). Such an address is judged by the one it carries.
fn embedded_v4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = address.segments();
    let v4 = |high: u16, low: u16| Ipv4Addr::from((u32::from(high) << 16) | u32::from(low));
    match s {
        [0, 0, 0, 0, 0, 0xffff, high, low] | [0x64, 0xff9b, 0, 0, 0, 0, high, low] => Some(v4(high, low)),
        [0x2002, high, low, ..] => Some(v4(high, low)),
        _ => None,
    }
}
