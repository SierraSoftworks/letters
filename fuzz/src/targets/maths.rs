//! TeX from anywhere: a formula is typeset or refused, and one that is typeset can be drawn and written.

use letters_math::{Cache, Style, mathml, typeset};
use letters_render::{Color, DisplayList, Point};

use crate::input::text;

pub fn run(data: &[u8]) {
    let source = text(data);
    let mut cache = Cache::new(4);
    for (display, links) in [(false, false), (true, true)] {
        let style = Style { size: 13.3, color: Color::rgb(0, 0, 0), display, links };
        if let Ok(formula) = typeset(&source, &style) {
            let sized = [formula.size.width, formula.size.height, formula.baseline];
            assert!(sized.iter().all(|length| length.is_finite()), "a formula has no size: {sized:?}");
            formula.paint(Point { x: 0.0, y: 0.0 }, &mut DisplayList::default());
            formula.svg(Color::rgb(0, 0, 0));
            let cached = cache.typeset(&source, &style).expect("what typesets once typesets again");
            assert!(*cached == formula, "a formula is typeset differently the second time");
        }
        let _ = mathml(&source, display);
    }
}

pub fn seeds() -> Vec<Vec<u8>> {
    let sources = [
        r"A = P \left(1 + \frac{r}{n}\right)^{nt}",
        r"\begin{aligned} a &= b + c \\ \sqrt[3]{x} &\le \sum_{i=0}^{n} x_i^2 \end{aligned}",
        r"\begin{pmatrix} 1 & 0 \\ 0 & 1 \end{pmatrix} \cdot \vec{v} = \href{https://example.com}{\text{v é}}",
        r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt\pi}{2} \quad \color{red}{\mathbb{R}} \overbrace{a+b}^{n} \binom{n}{k}",
        r"\begin{cases} x & \text{if } x > 0 \\ -x & \text{otherwise} \end{cases} \xrightarrow{f} \mathrm{H_2O}",
    ];
    sources.iter().map(|source| source.as_bytes().to_vec()).collect()
}
