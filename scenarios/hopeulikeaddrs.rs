use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bitcoin::p2p::address::{AddrV2, AddrV2Message};
use bitcoin::p2p::{message::NetworkMessage, ServiceFlags};
use bitcoin::Network;
use bitcoind_test_framework::common::{types::ConnectionType, ConnectionPool, DEFAULT_P2P_PORT};
use tokio::time::Instant;

const NUM_CONNECITONS: u32 = 100;
const SPAM_TIME_SECONDS: u64 = 15;

#[tokio::main]
async fn main() {
    let ip_addr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let local_host = SocketAddr::new(ip_addr, DEFAULT_P2P_PORT);
    let pool = ConnectionPool::new(Network::Regtest);
    println!("Starting conneciton pool");
    for _ in 0..NUM_CONNECITONS {
        pool.new_outbound(local_host, ConnectionType::V1, ServiceFlags::NONE, true)
            .await
            .unwrap();
    }
    println!("Established {NUM_CONNECITONS} connections");
    let start_time = Instant::now();
    let mut cnt = 0;
    println!("Starting ADDR spam");
    loop {
        if Instant::now().duration_since(start_time) > Duration::from_secs(SPAM_TIME_SECONDS) {
            println!("Done spamming ADDR. We spammed {cnt} bogus I2p ADDR in {SPAM_TIME_SECONDS} seconds!");
            break;
        }
        let message = AddrV2Message {
            time: 0,
            services: ServiceFlags::NONE,
            addr: AddrV2::I2p([0; 32]),
            port: 9999,
        };
        let fails = pool.send_all(NetworkMessage::AddrV2(vec![message])).await;
        cnt = cnt + NUM_CONNECITONS - fails.0.len() as u32;
    }
}
