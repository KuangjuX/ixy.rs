use std::collections::VecDeque;
use std::env;
use std::process;
use std::time::Instant;

use byteorder::{ByteOrder, LittleEndian};
use chrono::Local;
use ixy::memory::{alloc_pkt_batch, Mempool, Packet};
use ixy::*;
use simple_logger::SimpleLogger;

// number of packets sent simultaneously by our driver
const BATCH_SIZE: usize = 32;
// number of packets in our mempool
const NUM_PACKETS: usize = 2048;
// size of our packets
const PACKET_SIZE: usize = 1500;

const MB: usize = 1000 * 1000;
const GB: usize = 1000 * MB;
const MAX_BYTES: usize = 50 * GB;

const DST_MAC: [u8; 6] = [0x90, 0xe2, 0xba, 0x8c, 0x66, 0x88];

pub fn main() {
    SimpleLogger::new().init().unwrap();

    let mut args = env::args();
    args.next();

    let pci_addr = match args.next() {
        Some(arg) => arg,
        None => {
            eprintln!("Usage: cargo run --example generator <pci bus id>");
            process::exit(1);
        }
    };

    let mut dev = ixy_init(&pci_addr, 1, 1, 0).unwrap();

    #[rustfmt::skip]
    let mut pkt_data = [
        0x90, 0xe2, 0xba, 0x8c, 0x66, 0x88,         // dst MAC
        0x00, 0x16, 0x31, 0xff, 0xa6, 0x42,         // src MAC
        0x08, 0x00,                                 // ether type: IPv4
        0x45, 0x00,                                 // Version, IHL, TOS
        ((PACKET_SIZE - 14) >> 8) as u8,            // ip len excluding ethernet, high byte
        ((PACKET_SIZE - 14) & 0xFF) as u8,          // ip len excluding ethernet, low byte
        0x00, 0x00, 0x00, 0x00,                     // id, flags, fragmentation
        0x40, 0x11, 0x00, 0x00,                     // TTL (64), protocol (UDP), checksum
        0x0A, 0x00, 0x00, 0x01,                     // src ip (10.0.0.1)
        0x0A, 0x00, 0x00, 0x02,                     // dst ip (10.0.0.2)
        0x00, 0x2A, 0x05, 0x39,                     // src and dst ports (42 -> 1337)
        ((PACKET_SIZE - 20 - 14) >> 8) as u8,       // udp len excluding ip & ethernet, high byte
        ((PACKET_SIZE - 20 - 14) & 0xFF) as u8,     // udp len excluding ip & ethernet, low byte
        0x00, 0x00,                                 // udp checksum, optional
    ];

    pkt_data[0..6].clone_from_slice(&DST_MAC);
    pkt_data[6..12].clone_from_slice(&dev.get_mac_addr());

    let pool = Mempool::allocate(NUM_PACKETS, 0).unwrap();

    let mut dev_stats = Default::default();
    let mut dev_stats_old = Default::default();

    dev.reset_stats();

    dev.read_stats(&mut dev_stats);
    dev.read_stats(&mut dev_stats_old);

    let mut send_bytes = 0;
    let mut past_send_bytes = 0;
    let mut past_time = Local::now();

    loop {
        let mut buffer: VecDeque<Packet> = VecDeque::with_capacity(NUM_PACKETS);

        alloc_pkt_batch(&pool, &mut buffer, NUM_PACKETS, PACKET_SIZE);

        for p in buffer.iter_mut() {
            p[0..42].clone_from_slice(&pkt_data);
        }
        // re-fill our packet queue with new packets to send out
        alloc_pkt_batch(&pool, &mut buffer, BATCH_SIZE, PACKET_SIZE);

        dev.tx_batch_busy_wait(0, &mut buffer);

        send_bytes += BATCH_SIZE * PACKET_SIZE;

        let current_time = Local::now();
        if current_time.signed_duration_since(past_time).num_seconds() == 1 {
            let gb = ((send_bytes - past_send_bytes) * 8) / GB;
            let mb = (((send_bytes - past_send_bytes) * 8) % GB) / MB;
            let gib = (send_bytes - past_send_bytes) / GB;
            let mib = ((send_bytes - past_send_bytes) % GB) / MB;
            println!(
                "Transfer: {:03}.{:03}GBytes, Bandwidth: {:03}.{:03}Gbits/sec.",
                gib, mib, gb, mb
            );
            past_send_bytes = send_bytes;
            past_time = current_time;
        }

        if send_bytes >= MAX_BYTES {
            break;
        }
    }
}
