//! iSCSI session management
//!
//! Handles login, text negotiation, and SCSI command processing.

use super::pdu::{BasicHeaderSegment, Opcode, Pdu, ScsiStatus};
use crate::storage::BlockStorage;
use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::TcpStream;

const SECTOR_SIZE: usize = 512;

/// iSCSI session state
#[derive(Debug)]
pub struct Session {
    pub initiator_name: Option<String>,
    pub target_name: String,
    pub session_id: u16,
    pub cmd_sn: u32,
    pub exp_stat_sn: u32,
    pub max_cmd_sn: u32,
}

impl Session {
    pub fn new(target_name: String, session_id: u16) -> Self {
        Self {
            initiator_name: None,
            target_name,
            session_id,
            cmd_sn: 0,
            exp_stat_sn: 1,
            max_cmd_sn: 64,
        }
    }
}

/// Parse iSCSI text parameters (key=value pairs)
pub fn parse_text_params(data: &[u8]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(data);
    let mut params = HashMap::new();

    for line in text.split('\0') {
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            params.insert(key.to_string(), value.to_string());
        }
    }

    params
}

/// Format text parameters as null-terminated strings
pub fn format_text_params(params: &HashMap<String, String>) -> Vec<u8> {
    let mut data = Vec::new();
    for (key, value) in params {
        data.extend_from_slice(format!("{}={}\0", key, value).as_bytes());
    }
    data
}

/// Handle iSCSI login request
pub fn handle_login(
    pdu: &Pdu,
    session: &mut Session,
) -> io::Result<Pdu> {
    let params = parse_text_params(&pdu.data);

    log::debug!("Login parameters: {:?}", params);

    // Extract initiator name
    if let Some(name) = params.get("InitiatorName") {
        session.initiator_name = Some(name.clone());
    }

    // Build response parameters
    let mut response_params = HashMap::new();
    response_params.insert("TargetName".to_string(), session.target_name.clone());
    response_params.insert("TargetPortalGroupTag".to_string(), "1".to_string());

    // Auth parameters - accept without authentication for simplicity
    if params.contains_key("AuthMethod") {
        response_params.insert("AuthMethod".to_string(), "None".to_string());
    }

    // Session parameters
    response_params.insert("MaxRecvDataSegmentLength".to_string(), "262144".to_string());
    response_params.insert("MaxBurstLength".to_string(), "262144".to_string());
    response_params.insert("FirstBurstLength".to_string(), "65536".to_string());
    response_params.insert("DefaultTime2Wait".to_string(), "2".to_string());
    response_params.insert("DefaultTime2Retain".to_string(), "20".to_string());
    response_params.insert("IFMarker".to_string(), "No".to_string());
    response_params.insert("OFMarker".to_string(), "No".to_string());
    response_params.insert("MaxConnections".to_string(), "1".to_string());
    response_params.insert("InitialR2T".to_string(), "Yes".to_string());
    response_params.insert("ImmediateData".to_string(), "Yes".to_string());
    response_params.insert("DataPDUInOrder".to_string(), "Yes".to_string());
    response_params.insert("DataSequenceInOrder".to_string(), "Yes".to_string());
    response_params.insert("ErrorRecoveryLevel".to_string(), "0".to_string());

    let response_data = format_text_params(&response_params);

    let mut response = Pdu::new(Opcode::LoginResponse);
    response.bhs.flags = 0x80 | (pdu.bhs.flags & 0x03); // Transit to next stage
    response.bhs.data_segment_length = response_data.len() as u32;
    response.bhs.initiator_task_tag = pdu.bhs.initiator_task_tag;
    response.bhs.exp_stat_sn = session.exp_stat_sn;
    response.bhs.max_cmd_sn = session.max_cmd_sn;
    response.bhs.specific[0] = (pdu.bhs.specific[0] & 0x03) | 0x80; // Current stage + transit
    response.data = response_data;

    // Update session state
    session.exp_stat_sn += 1;

    Ok(response)
}

/// Handle SCSI Read command (READ(10), READ(16))
pub fn handle_scsi_read<S: BlockStorage>(
    pdu: &Pdu,
    session: &mut Session,
    storage: &mut S,
) -> io::Result<Vec<Pdu>> {
    // Parse SCSI CDB from specific fields
    let cdb = &pdu.data;
    if cdb.is_empty() {
        return Ok(vec![create_scsi_response(pdu, session, ScsiStatus::CheckCondition)]);
    }

    let opcode = cdb[0];
    let (lba, transfer_length) = match opcode {
        0x28 => {
            // READ(10)
            let lba = u32::from_be_bytes([cdb[2], cdb[3], cdb[4], cdb[5]]) as u64;
            let transfer_length = u16::from_be_bytes([cdb[7], cdb[8]]) as u32;
            (lba, transfer_length)
        }
        0x88 => {
            // READ(16)
            let lba = u64::from_be_bytes([
                cdb[2], cdb[3], cdb[4], cdb[5], cdb[6], cdb[7], cdb[8], cdb[9],
            ]);
            let transfer_length = u32::from_be_bytes([cdb[10], cdb[11], cdb[12], cdb[13]]);
            (lba, transfer_length)
        }
        _ => {
            log::warn!("Unsupported SCSI read opcode: 0x{:02x}", opcode);
            return Ok(vec![create_scsi_response(pdu, session, ScsiStatus::CheckCondition)]);
        }
    };

    log::debug!("SCSI READ: LBA={}, length={} sectors", lba, transfer_length);

    // Read data from storage
    let byte_count = (transfer_length as usize) * SECTOR_SIZE;
    let mut data = vec![0u8; byte_count];

    if let Err(e) = storage.read_sectors(lba, transfer_length as u8, &mut data) {
        log::error!("Storage read error: {}", e);
        return Ok(vec![create_scsi_response(pdu, session, ScsiStatus::CheckCondition)]);
    }

    // Create Data-In PDU
    let mut data_in = Pdu::new(Opcode::ScsiDataIn);
    data_in.bhs.flags = 0x81; // Final + Status
    data_in.bhs.data_segment_length = data.len() as u32;
    data_in.bhs.initiator_task_tag = pdu.bhs.initiator_task_tag;
    data_in.bhs.exp_cmd_sn = session.cmd_sn + 1;
    data_in.bhs.max_cmd_sn = session.max_cmd_sn;
    data_in.bhs.specific[0] = ScsiStatus::Good as u8;
    data_in.data = data;

    session.exp_stat_sn += 1;

    Ok(vec![data_in])
}

/// Handle SCSI Write command (WRITE(10), WRITE(16))
pub fn handle_scsi_write<S: BlockStorage>(
    pdu: &Pdu,
    session: &mut Session,
    storage: &mut S,
) -> io::Result<Pdu> {
    let cdb = &pdu.data[..16]; // First 16 bytes are CDB
    let opcode = cdb[0];

    let (lba, transfer_length) = match opcode {
        0x2a => {
            // WRITE(10)
            let lba = u32::from_be_bytes([cdb[2], cdb[3], cdb[4], cdb[5]]) as u64;
            let transfer_length = u16::from_be_bytes([cdb[7], cdb[8]]) as u32;
            (lba, transfer_length)
        }
        0x8a => {
            // WRITE(16)
            let lba = u64::from_be_bytes([
                cdb[2], cdb[3], cdb[4], cdb[5], cdb[6], cdb[7], cdb[8], cdb[9],
            ]);
            let transfer_length = u32::from_be_bytes([cdb[10], cdb[11], cdb[12], cdb[13]]);
            (lba, transfer_length)
        }
        _ => {
            log::warn!("Unsupported SCSI write opcode: 0x{:02x}", opcode);
            return Ok(create_scsi_response(pdu, session, ScsiStatus::CheckCondition));
        }
    };

    log::debug!("SCSI WRITE: LBA={}, length={} sectors", lba, transfer_length);

    // Write data (comes after CDB in immediate data)
    let data_offset = 16;
    let write_data = &pdu.data[data_offset..];

    if let Err(e) = storage.write_sectors(lba, transfer_length as u8, write_data) {
        log::error!("Storage write error: {}", e);
        return Ok(create_scsi_response(pdu, session, ScsiStatus::CheckCondition));
    }

    Ok(create_scsi_response(pdu, session, ScsiStatus::Good))
}

/// Create SCSI response PDU
fn create_scsi_response(request: &Pdu, session: &Session, status: ScsiStatus) -> Pdu {
    let mut response = Pdu::new(Opcode::ScsiResponse);
    response.bhs.flags = 0x80; // Final bit
    response.bhs.initiator_task_tag = request.bhs.initiator_task_tag;
    response.bhs.exp_cmd_sn = session.cmd_sn + 1;
    response.bhs.max_cmd_sn = session.max_cmd_sn;
    response.bhs.specific[0] = status as u8;
    response
}
