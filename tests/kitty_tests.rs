use tuiui::kitty::{b64, delete, place, transmit, transmit_b64};

#[test]
fn base64_matches_known_vectors() {
    assert_eq!(b64(b""), "");
    assert_eq!(b64(b"M"), "TQ==");
    assert_eq!(b64(b"Ma"), "TWE=");
    assert_eq!(b64(b"Man"), "TWFu");
}

#[test]
fn place_and_delete_strings() {
    assert_eq!(place(7, 10, 4), "\x1b_Ga=p,i=7,c=10,r=4,q=2\x1b\\");
    assert_eq!(delete(7), "\x1b_Ga=d,d=i,i=7,q=2\x1b\\");
}

#[test]
fn transmit_one_chunk_for_small_payload() {
    assert_eq!(transmit(3, b"Man"), "\x1b_Gf=100,a=t,t=d,i=3,q=2,m=0;TWFu\x1b\\");
}

#[test]
fn transmit_equals_transmit_b64_of_b64() {
    assert_eq!(transmit(3, b"Man"), transmit_b64(3, "TWFu"));
}

#[test]
fn transmit_chunks_large_payload() {
    let big = vec![0u8; 4096]; // → ~5462 base64 chars → 2 chunks
    let s = transmit(1, &big);
    assert!(s.matches("\x1b_G").count() >= 2);
    assert!(s.contains("m=1"));
    assert!(s.contains("m=0;"));
    assert!(s.ends_with("\x1b\\"));
}
