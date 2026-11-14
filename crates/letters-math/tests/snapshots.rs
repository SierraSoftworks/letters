//! Whole formulas compared with stored pictures, drawn by the CPU backend as the editor draws them.

mod common;

use common::*;
use letters_math::Formula;

/// The text size the scenes sit beside.
const TEXT: f32 = 16.0;

fn scene(style: &letters_math::Style, sources: &[&str]) -> Vec<Formula> {
    sources.iter().map(|source| set(source, style)).collect()
}

#[test]
fn inline_formulas_match_their_snapshot() {
    let formulas = scene(
        &inline(TEXT * letters_math::TEXT_RATIO),
        &[
            r"e^{i\pi} + 1 = 0",
            r"\sum_{i=0}^{n} i = \frac{n(n+1)}{2}",
            r"x \in \mathbb{R},\ \alpha \le \beta \ne \gamma \implies f\colon A \to B",
            r"\sqrt[3]{x^2 + 1} \approx \lim_{h \to 0} \frac{f(x+h) - f(x)}{h}",
        ],
    );

    assert_snapshot("inline", &formulas, 1.0);
}

#[test]
fn display_formulas_match_their_snapshot() {
    let formulas = scene(
        &display(TEXT * letters_math::TEXT_RATIO),
        &[
            r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
            r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2} \qquad \prod_{k=1}^{n} k = n!",
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix} \begin{bmatrix} 1 & 0 \\ 0 & 1 \end{bmatrix} = \left\{ \frac{\frac{a}{b}}{\frac{c}{d}} \right\}",
            r"|x| = \begin{cases} x & \text{if } x \ge 0 \\ -x & \text{otherwise} \end{cases}",
        ],
    );

    assert_snapshot("display@2x", &formulas, 2.0);
}

#[test]
fn marks_faces_and_colours_match_their_snapshot() {
    let formulas = scene(
        &display(TEXT * letters_math::TEXT_RATIO),
        &[
            r"\overrightarrow{AB} + \underbrace{a + b + c}_{n} + \widehat{abc} + \vec{v} \xrightarrow{\text{heat}} \overline{z}",
            r"\cancel{x} + \sout{y} + \oiint + \bcancel{abc} + \boxed{E = mc^2}",
            r"\mathbb{R}\,\mathcal{L}\,\mathfrak{g}\,\mathscr{F}\,\mathsf{x}\,\mathtt{y}\,\mathbf{z}\,\boldsymbol{\alpha}",
            r"\colorbox{yellow}{a} + \textcolor{#c2255c}{x + y} + {\color{teal} z} + \text{naïve привет}",
            r"\begin{array}{c:c|c} a & b & c \\ \hline d & e & f \\ \hdashline g & h & i \end{array}",
        ],
    );

    assert_snapshot("marks", &formulas, 1.0);
}
