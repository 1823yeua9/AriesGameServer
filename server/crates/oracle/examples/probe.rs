//! read-only live check: find the client, decode GObjects, derive the FName
//! key, scan for the session keys. prints fingerprints only.

use aries_oracle::keys::{fingerprint, read_handler_keys};
use aries_oracle::{GObjects, NamePool, Process, find_processes};

fn main() {
    let found = find_processes("VALORANT-Win64-Shipping.exe");
    let Some(client) = found.first() else {
        println!("no client running");
        return;
    };
    println!("client pid {} base {:#x} path {:?}", client.pid, client.image_base, client.path);
    let process = Process::open(client.pid, client.image_base).expect("open");
    let objects = GObjects::read(&process).expect("gobjects");
    println!("gobjects: {} chunks, {} elements", objects.chunks.len(), objects.num_elements);
    let engine = objects.engine(&process).expect("engine");
    let mut names = NamePool::new(&process, &objects).expect("name key");
    println!("engine {:#x} = {} ({})", engine, names.object_name(&process, engine).unwrap(), names.class_name(&process, engine).unwrap());
    let started = std::time::Instant::now();
    match read_handler_keys(client.pid, client.image_base, 30.0) {
        Ok(keys) => println!("handler {:#x} in {:?}: server-send fingerprint {} client-send fingerprint {}", keys.handler, started.elapsed(), fingerprint(&keys.server_send), fingerprint(&keys.client_send)),
        Err(error) => println!("key scan failed: {error}"),
    }
}
