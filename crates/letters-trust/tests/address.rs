use std::net::IpAddr;

use letters_trust::is_public;

#[test]
fn only_the_public_internet_is_public() {
    let cases = [
        ("93.184.216.34", true),
        ("8.8.8.8", true),
        ("2606:4700:4700::1111", true),
        ("2a00:1450:4001:81b::200e", true),
        // This machine.
        ("127.0.0.1", false),
        ("127.255.255.254", false),
        ("0.0.0.0", false),
        ("::1", false),
        ("::", false),
        // Private and shared networks.
        ("10.0.0.1", false),
        ("172.16.0.1", false),
        ("172.31.255.255", false),
        ("192.168.1.1", false),
        ("100.64.0.1", false),
        ("100.127.255.255", false),
        ("fc00::1", false),
        ("fd7a:115c:a1e0::1", false),
        // Link-local, where cloud metadata lives.
        ("169.254.169.254", false),
        ("fe80::1", false),
        // Multicast, broadcast, reserved and documentation.
        ("224.0.0.1", false),
        ("255.255.255.255", false),
        ("240.0.0.1", false),
        ("192.0.2.1", false),
        ("198.18.0.1", false),
        ("2001:db8::1", false),
        ("ff02::1", false),
        // Just outside the private ranges.
        ("11.0.0.1", true),
        ("172.32.0.1", true),
        ("100.128.0.1", true),
        ("192.169.0.1", true),
        // IPv6 that stands for an IPv4 address is judged by it.
        ("::ffff:127.0.0.1", false),
        ("::ffff:10.0.0.1", false),
        ("::ffff:93.184.216.34", true),
        ("64:ff9b::a9fe:a9fe", false),
        ("64:ff9b::5db8:d822", true),
        ("2002:7f00:1::1", false),
        ("2002:c0a8:101::1", false),
        ("::127.0.0.1", false),
        ("2001::1", false),
    ];
    for (address, public) in cases {
        let parsed: IpAddr = address.parse().expect(address);
        assert_eq!(is_public(parsed), public, "{address}");
    }
}
