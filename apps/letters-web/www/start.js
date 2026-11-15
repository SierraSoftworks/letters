// Starts the editor in the page's canvas.

import init, { Letters } from "./letters.js";

await init();
// The files the editor fetches are beside this one, wherever the page itself is. The console and the tests
// reach the editor here.
window.letters = await Letters.start(document.getElementById("letters"), new URL(".", import.meta.url).href);
