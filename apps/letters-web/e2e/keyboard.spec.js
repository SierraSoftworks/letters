// What a keyboard that corrects, capitalises and predicts is given to read, and what it may do with it: the
// hidden input holds the text around the caret, and an edit to any of it reaches the document.

import { expect, test } from "@playwright/test";

import { ready, texts } from "./page.js";

const input = (page) => page.locator("textarea");
const last = async (page) => (await texts(page)).at(-1);
/** The hidden input's selection, as the two ends of it. */
const selection = (page) => input(page).evaluate((input) => [input.selectionStart, input.selectionEnd]);
/** Presses the control called `name`, where the mirror says it is. */
const click = async (page, name) => {
  const box = await page.getByRole("button", { name, exact: true }).first().boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
};

test.beforeEach(({ page }) => ready(page));

test("the hidden input holds the text around the caret, with its selection where the editor's is", async ({ page }) => {
  await page.keyboard.type("One two");
  await expect(input(page)).toHaveValue(" One two");
  expect(await selection(page)).toEqual([8, 8]);
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("Shift+ArrowLeft");
  await expect.poll(() => selection(page)).toEqual([6, 7]);
  // Sentences are capitalised and words mended from what it holds.
  await expect(input(page)).toHaveAttribute("autocapitalize", "sentences");
  await expect(input(page)).toHaveAttribute("autocorrect", "on");
});

test("code is typed as it is meant, with none of it for a keyboard to mend", async ({ page }) => {
  await page.keyboard.type("```");
  await page.keyboard.press("Enter");
  await page.keyboard.type("let teh = 1;");
  expect(await last(page)).toBe("let teh = 1;");
  await expect(input(page)).toHaveValue(" ");
  await expect(input(page)).toHaveAttribute("autocapitalize", "off");
  await expect(input(page)).toHaveAttribute("autocorrect", "off");
});

test("a field of the window is typed in without the document's text", async ({ page }) => {
  await page.keyboard.type("One two");
  await click(page, "Letters in the browser");
  await expect(page.getByRole("textbox", { name: "Document name" })).toBeAttached();
  await expect(input(page)).toHaveValue(" ");
  await page.keyboard.type("x");
  expect(await last(page)).toBe("One two");
});

test("a word the keyboard mends where it stands is mended in the document", async ({ page }) => {
  await page.keyboard.type("I saw teh cat");
  // The keyboard aims at the word, and asks before it replaces it.
  await input(page).evaluate((input) => {
    input.setSelectionRange(7, 10);
    input.dispatchEvent(new InputEvent("beforeinput", { inputType: "insertReplacementText", data: "the", cancelable: true }));
  });
  await expect.poll(() => last(page)).toBe("I saw the cat");
  await expect(input(page)).toHaveValue(" I saw the cat");
  // The caret is still at the end of the line, where typing carries on.
  await expect.poll(() => selection(page)).toEqual([14, 14]);
  await page.keyboard.type("!");
  expect(await last(page)).toBe("I saw the cat!");
});

test("a correction nothing announced is read from what the hidden input then holds", async ({ page }) => {
  await page.keyboard.type("I saw teh");
  // A soft keyboard mends the word as the space after it is typed, and says only that the input changed.
  await input(page).evaluate((input) => {
    input.value = " I saw the ";
    input.setSelectionRange(11, 11);
    input.dispatchEvent(new InputEvent("input", { inputType: "insertText", data: " " }));
  });
  await expect.poll(() => last(page)).toBe("I saw the ");
  await page.keyboard.type("cat");
  expect(await last(page)).toBe("I saw the cat");
});

test("a word the keyboard takes back into its composition is not written twice", async ({ page }) => {
  await page.keyboard.type("hello");
  // The keyboard composes on from the word before the caret, and ends with the whole word.
  await input(page).evaluate((input) => {
    input.dispatchEvent(new CompositionEvent("compositionstart", { data: "" }));
    input.value = " hellow";
    input.setSelectionRange(7, 7);
    input.dispatchEvent(new InputEvent("input", { inputType: "insertCompositionText", data: "hellow", isComposing: true }));
    input.dispatchEvent(new CompositionEvent("compositionend", { data: "hellow" }));
  });
  await expect.poll(() => last(page)).toBe("hellow");
  await expect(input(page)).toHaveValue(" hellow");
});
