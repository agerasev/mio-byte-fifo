extern crate mio_byte_fifo;

use std::io::{Read, Write};


fn main() {
    let (mut p, mut c) = mio_byte_fifo::create(16);

    let n = p.write(&[0, 1, 254, 255]).unwrap();
    println!("{} bytes written", n);

    let mut buf = [0; 4];
    let n = c.read(&mut buf).unwrap();
    println!("{} bytes read: {:?}", n, buf);
}
