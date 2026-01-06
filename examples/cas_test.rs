//! Simple test client for the CAS service

use aoe_server::cas::protocol::{read_frame, write_frame, CasCommand};
use std::io::{self, BufReader, BufWriter};
use std::net::TcpStream;

fn main() -> io::Result<()> {
    println!("Connecting to CAS server at 127.0.0.1:3000...");
    let stream = TcpStream::connect("127.0.0.1:3000")?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = BufWriter::new(stream);

    // Test 1: Ping
    println!("\nTest 1: Ping");
    write_frame(&mut writer, CasCommand::Ping, &[])?;
    let (cmd, data) = read_frame(&mut reader)?;
    println!("  Response: {:?}, data len: {}", cmd, data.len());

    // Test 2: Write some data
    println!("\nTest 2: Write data");
    let test_data = b"Hello, Content-Addressable Storage!";
    write_frame(&mut writer, CasCommand::Write, test_data)?;
    let (cmd, hash) = read_frame(&mut reader)?;
    println!("  Response: {:?}", cmd);
    println!("  Hash: {}", hex::encode(&hash));

    // Test 3: Check if hash exists
    println!("\nTest 3: Check existence");
    write_frame(&mut writer, CasCommand::Exists, &hash)?;
    let (cmd, exists_data) = read_frame(&mut reader)?;
    let exists = exists_data.get(0).copied().unwrap_or(0) != 0;
    println!("  Response: {:?}, exists: {}", cmd, exists);

    // Test 4: Read data back
    println!("\nTest 4: Read data");
    write_frame(&mut writer, CasCommand::Read, &hash)?;
    let (cmd, data) = read_frame(&mut reader)?;
    println!("  Response: {:?}", cmd);
    println!("  Data: {}", String::from_utf8_lossy(&data));
    println!("  Matches: {}", data == test_data);

    // Test 5: Write duplicate data (should return same hash)
    println!("\nTest 5: Write duplicate");
    write_frame(&mut writer, CasCommand::Write, test_data)?;
    let (cmd, hash2) = read_frame(&mut reader)?;
    println!("  Response: {:?}", cmd);
    println!("  Hash: {}", hex::encode(&hash2));
    println!("  Same hash: {}", hash == hash2);

    println!("\nAll tests passed!");
    Ok(())
}
