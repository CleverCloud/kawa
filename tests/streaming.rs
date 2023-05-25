use std::{hint::black_box, io::Write};

use kawa::{h1, Buffer, Kawa, Kind, SliceBuffer};

#[test]
fn bench_long() {
    const REQ_LONG: &'static [u8] = b"\
GET /wp-content/uploads/2010/03/hello-kitty-darth-vader-pink.jpg HTTP/1.1\r\n\
Host: www.kittyhell.com\r\n\
User-Agent: Mozilla/5.0 (Macintosh; U; Intel Mac OS X 10.6; ja-JP-mac; rv:1.9.2.3) Gecko/20100401 Firefox/3.6.3 Pathtraq/0.9\r\n\
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\n\
Accept-Language: ja,en-us;q=0.7,en;q=0.3\r\n\
Accept-Encoding: gzip,deflate\r\n\
Accept-Charset: Shift_JIS,utf-8;q=0.7,*;q=0.7\r\n\
Keep-Alive: 115\r\n\
Connection: keep-alive\r\n\
Cookie: wp_ozh_wsa_visits=2; wp_ozh_wsa_visit_lasttime=xxxxxxxxxx; foo; ==bar=; __utma=xxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.x; __utmz=xxxxxxxxx.xxxxxxxxxx.x.x.utmccn=(referral)|utmcsr=reader.livedoor.com|utmcct=/reader/|utmcmd=referral\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.blocks.reserve(16);
    req.detached.jar.reserve(16);

    for _ in 0..10000 {
        req.clear();
        for char in REQ_LONG {
            req.storage.write(&[*char]).expect("write");
            black_box(h1::parse(&mut req, &mut h1::NoCallbacks));
        }
        if !req.is_main_phase() {
            kawa::debug_kawa(&req);
            assert!(false);
        }
    }
}

#[test]
fn bench_short() {
    const REQ_SHORT: &'static [u8] = b"\
GET / HTTP/1.0\r\n\
Host: example.com\r\n\
Connection: close\r\n\r\n";

    let mut buffer = vec![0; 512];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.blocks.reserve(16);

    for _ in 0..10000 {
        req.clear();
        for char in REQ_SHORT {
            req.storage.write(&[*char]).expect("write");
            black_box(h1::parse(&mut req, &mut h1::NoCallbacks));
        }
        if !req.is_main_phase() {
            kawa::debug_kawa(&req);
            assert!(false);
        }
    }
}
