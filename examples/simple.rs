extern crate mio_byte_fifo;

use std::io::{Read, Write};


fn main() {
    let (mut producer, mut consumer) = mio_byte_fifo::create(16);

    let data = [0, 1, 254, 255];
    let n = producer.write(&data).unwrap();
    println!("written {} bytes: {:?}", n, data);

    let mut buf = [0; 8];
    let n = consumer.read(&mut buf).unwrap();
    println!("read    {} bytes: {:?}", n, &buf[0..n]);

    assert_eq!(data, buf[0..n]);
}
