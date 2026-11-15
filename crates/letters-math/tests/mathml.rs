//! A formula's structure as MathML: the elements a reader of it leans on, and what it does with a source it
//! cannot write.

use letters_math::{commands, environments, mathml};

/// The elements inside the `math` element of a formula that is known to be good.
fn inner(source: &str, display: bool) -> String {
    let xml = mathml(source, display).unwrap_or_else(|error| panic!("{source}: {error}"));
    let tree = roxmltree::Document::parse(&xml).unwrap_or_else(|error| panic!("{source} is not XML: {error}\n{xml}"));
    assert_eq!(tree.root_element().tag_name().name(), "math", "{xml}");
    assert_eq!(tree.root_element().attribute("display") == Some("block"), display, "{xml}");
    let start = xml.find('>').expect("the root opens") + 1;
    xml[start..xml.len() - "</math>".len()].to_owned()
}

#[test]
fn a_formula_is_written_as_its_structure() {
    // (source, what its MathML holds)
    let cases = [
        ("x", "<mi>x</mi>"),
        ("3.14", "<mn>3.14</mn>"),
        (r"\alpha + 1", "<mi>α</mi><mo>+</mo><mn>1</mn>"),
        (r"a \neq b", "<mi>a</mi><mo>≠</mo><mi>b</mi>"),
        ("x^2", "<msup><mi>x</mi><mn>2</mn></msup>"),
        ("x_i^2", "<msubsup><mi>x</mi><mi>i</mi><mn>2</mn></msubsup>"),
        (r"\frac{a}{b}", "<mfrac><mi>a</mi><mi>b</mi></mfrac>"),
        (r"\binom{n}{k}", r#"<mfrac linethickness="0"><mi>n</mi><mi>k</mi></mfrac>"#),
        (r"\sqrt{x}", "<msqrt><mi>x</mi></msqrt>"),
        (r"\sqrt[3]{x}", "<mroot><mi>x</mi><mn>3</mn></mroot>"),
        (r"\hat{x}", r#"<mover accent="true"><mi>x</mi>"#),
        (r"\mathbb{R}", r#"<mi mathvariant="double-struck">R</mi>"#),
        (r"\mathrm{d}x", r#"<mi mathvariant="normal">d</mi><mi>x</mi>"#),
        (r"\text{if } x", "<mtext>if\u{a0}</mtext><mi>x</mi>"),
        (r"\sin x", r#"<mi mathvariant="normal">sin</mi>"#),
        (
            r"\left( x \right]",
            r#"<mo fence="true" form="prefix" stretchy="true">(</mo><mi>x</mi><mo fence="true" form="postfix" stretchy="true">]</mo>"#,
        ),
        (r"\begin{matrix} a & b \\ c & d \end{matrix}", "<mtr><mtd><mi>a</mi></mtd><mtd><mi>b</mi></mtd></mtr><mtr>"),
        (r"\begin{cases} a \\ b \end{cases}", r#"<mtable columnalign="left left">"#),
        (r"\boxed{x}", r#"<menclose notation="box">"#),
        (r"a \, b", r#"<mi>a</mi><mspace width="0.167em"/><mi>b</mi>"#),
        ("a < b \\& c", "<mo>&lt;</mo><mi>b</mi><mo>&amp;</mo>"),
    ];
    for (source, expected) in cases {
        let written = inner(source, false);
        assert!(written.contains(expected), "{source} is written as {written}");
    }
}

#[test]
fn limits_sit_above_and_below_only_in_a_display() {
    let cases = [
        (r"\sum_{i=1}^{n} i", "<msubsup><mo>∑</mo>", "<munderover><mo>∑</mo>"),
        (r"\lim_{x \to 0} x", "<msub><mi", "<munder><mi"),
        // An integral keeps its limits beside it, and a brace its label under it.
        (r"\int_0^1 x", "<msubsup><mo>∫</mo>", "<msubsup><mo>∫</mo>"),
        (r"\underbrace{a}_{n}", "<munder><munder", "<munder><munder"),
    ];
    for (source, inline, display) in cases {
        assert!(inner(source, false).contains(inline), "{source} in a line: {}", inner(source, false));
        assert!(inner(source, true).contains(display), "{source} as a display: {}", inner(source, true));
    }
}

#[test]
fn everything_the_engine_offers_is_written_or_refused_by_name() {
    let mut refused = Vec::new();
    for row in commands().iter().chain(environments()) {
        match mathml(row.sample, true) {
            Ok(_) => assert!(!inner(row.sample, false).is_empty(), "{} is written as nothing", row.command),
            Err(error) => refused.push((row.command, error.message)),
        }
    }
    // MathML has no element for a diagram or a proof tree.
    for (command, reason) in &refused {
        assert!(reason.contains("diagram") || reason.contains("proof tree"), "{command}: {reason}");
    }
    assert!(refused.len() <= 4, "{refused:?}");
}

#[test]
fn a_source_that_cannot_be_written_says_why() {
    // (source, a word of the reason, whether the place is known)
    let cases = [
        (r"\frac{a}{", "end", true),
        (r"\nosuchcommand", "nosuchcommand", true),
        (r"\begin{CD} A @>a>> B \end{CD}", "diagram", false),
    ];
    for (source, reason, placed) in cases {
        let error = mathml(source, true).expect_err(source);
        assert!(error.message.contains(reason), "{source}: {error}");
        assert_eq!(error.span.is_some(), placed, "{source}: {:?}", error.span);
        assert!(error.span.is_none_or(|span| source.get(span).is_some()), "{source}: the place is in the source");
    }
}
