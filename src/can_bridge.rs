use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use heapless::Vec;

const QUEUE_SIZE: usize = 32;

#[derive(Clone, Copy)]
pub struct BridgeCanFrame {
    pub id: u32,
    pub leng: u8,
    pub data: [u8; 8],
}

pub static CAN_TO_HOST_CHANNEL: Channel<CriticalSectionRawMutex, BridgeCanFrame, QUEUE_SIZE> = Channel::new();
pub static HOST_TO_CAN_CHANNEL: Channel<CriticalSectionRawMutex, BridgeCanFrame, QUEUE_SIZE> = Channel::new();


#[embassy_executor::task]
pub async fn can_rx(mut rx: embassy_stm32::fdcan::Rx<'static, embassy_stm32::peripherals::FDCAN1>) {
    loop {
        match rx.read().await {
            Ok((frame, _ts)) => {
                let raw_id = match frame.header.id {
                    embassy_stm32::fdcan::FrameFormat::Standard(id) => id.as_raw() as u32,
                    embassy_stm32::fdcan::FrameFormat::Extended(id) => id.as_raw(),
                };

                let leng = frame.header.leng.min(8);
                let mut data = [0u8; 8];
                data[..leng as usize].copy_from_slice(&frame.data[..leng as usize]);

                let bridge_frame = BridgeCanFrame {
                    id: raw_id,
                    leng,
                    data,
                };

                let _ = CAN_TO_HOST_CHANNEL.send(bridge_frame).await;
            }
            Err(_) => {

            }
        }
    }
}


#[embassy_executor::task]
pub async fn host_rx(
    mut reader: embassy_usb::class::cdc_acm::Receiver<'static, embassy_usb::driver::Driver<'static>>
) {
    let mut buf = [0u8; 64];
    let mut parse_buf: Vec<u8, 64> = Vec::new(); 

    loop {
        match reader.read_packet(&mut buf).await {
            Ok(n) => {
                for &byte in &buf[..n] {
                    if parse_buf.is_full() {
                        parse_buf.remove(0); 
                    }
                    let _ = parse_buf.push(byte);

                    while parse_buf.len() >= 14 {
                        if parse_buf[0] == 0xAA {
                            let id = u32::from_le_bytes([parse_buf[1], parse_buf[2], parse_buf[3], parse_buf[4]]);
                            let leng = parse_buf[5].min(8);
                            
                            let mut data = [0u8; 8];
                            data.copy_from_slice(&parse_buf[6..14]);

                            let frame = BridgeCanFrame { id, leng, data };
                            
                            let _ = HOST_TO_CAN_CHANNEL.send(frame).await;

                            parse_buf.drain(0..14);
                        } else {
                            parse_buf.remove(0);
                        }
                    }
                }
            }
            Err(_) => {
                break;
            }
        }
    }
}


#[embassy_executor::task]
pub async fn can_tx(
    mut tx: embassy_stm32::fdcan::Tx<'static, embassy_stm32::peripherals::FDCAN1>
) {
    loop {
        let frame = HOST_TO_CAN_CHANNEL.receive().await;

        if let Some(std_id) = embassy_stm32::fdcan::StandardId::new(frame.id as u16) {
            if let Some(tx_frame) = embassy_stm32::fdcan::TxFrame::new_standard(std_id, &frame.data[..frame.leng as usize]) {
                let _ = tx.write(&tx_frame).await;
            }
        }
    }
}