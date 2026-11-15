//! A formula crosses as bytes to where the same faces are held, and arrives as it left.

mod common;

use common::*;
use letters_math::{Formula, typeset_mapped};

const SOURCES: [&str; 6] = [
    "",
    r"\frac{a}{b} + \sqrt[3]{x^2}",
    r"\left( \sum_{i=0}^{n} \mathbb{R} \right) \cancel{x}",
    r"\textcolor{red}{x} \colorbox{yellow}{y} \begin{array}{c:c} a & b \\ \hdashline c & d \end{array}",
    r"\text{по-русски} \mathfrak{g} \mathcal{L} \mathsf{s} \mathtt{t}",
    r"\begin{pmatrix} a & b \\ c & d \end{pmatrix} \underbrace{x + y}_{z}",
];

#[test]
fn a_formula_read_from_its_bytes_is_the_formula() {
    for source in SOURCES {
        for style in [inline(16.0), display(20.0)] {
            let plain = set(source, &style);
            assert_eq!(Formula::from_bytes(&plain.to_bytes()), Some(plain), "{source}");

            // What traces it to its source crosses with it.
            let mapped = typeset_mapped(source, &style).expect("a good source");
            assert_eq!(Formula::from_bytes(&mapped.to_bytes()), Some(mapped), "{source}, mapped");
        }
    }
}

#[test]
fn bytes_that_are_no_formula_are_refused() {
    let bytes = set(r"\frac{a}{b} + \sqrt{x}", &inline(16.0)).to_bytes();
    assert_eq!(Formula::from_bytes(&[]), None);
    for cut in (0..bytes.len()).step_by(7) {
        assert_eq!(Formula::from_bytes(&bytes[..cut]), None, "cut at {cut}");
    }
    let long = [&bytes[..], &[0]].concat();
    assert_eq!(Formula::from_bytes(&long), None, "a byte too many");
    // A count of more than the bytes could hold asks for no memory.
    let vast = [&bytes[..12], &[0xff; 4], &bytes[16..]].concat();
    assert_eq!(Formula::from_bytes(&vast), None, "a count of billions");
}
