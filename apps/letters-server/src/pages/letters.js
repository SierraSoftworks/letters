// The server's own behaviour on top of yewi's: a right click on a row of a file table, or the menu key
// with the row's name in focus, opens the row's menu where the pointer is. A row with no menu keeps the
// browser's own.
(() => {
  const AT = "lt-context";
  const menuOf = (row) => row.querySelector(".lt-file__more [data-yi-popover] > [popover]");
  const show = (surface, x, y) => {
    if (surface.matches(":popover-open")) surface.hidePopover();
    surface.classList.add(AT);
    surface.showPopover();
    // Kept on the page, however near its edge the pointer is.
    const { width, height } = surface.getBoundingClientRect();
    const fit = (at, size, room) => Math.max(4, Math.min(at, room - size - 4));
    surface.style.left = `${fit(x, width, innerWidth)}px`;
    surface.style.top = `${fit(y, height, innerHeight)}px`;
    surface.querySelector("[role^=menuitem]:not([aria-disabled=true])")?.focus();
  };
  // Whether a button of the pointer is held. A Mac asks for the menu as the button goes down, and the
  // browser closes a popover when a button that went down outside it comes up outside it: so the menu
  // waits for the button to come up.
  let held = false;
  let waiting;
  const released = () => {
    held = false;
    const shown = waiting;
    waiting = undefined;
    // After the browser has done with the release.
    if (shown) setTimeout(shown);
  };
  document.addEventListener("pointerdown", () => { held = true; waiting = undefined; }, true);
  document.addEventListener("pointerup", released, true);
  document.addEventListener("pointercancel", released, true);
  document.addEventListener("contextmenu", (event) => {
    const row = event.target instanceof Element && event.target.closest("tr.yi-table__row");
    const surface = row && menuOf(row);
    // A right click in a menu that is open keeps what it shows.
    if (!surface || surface.contains(event.target)) return;
    event.preventDefault();
    const shown = () => show(surface, event.clientX, event.clientY);
    if (held) waiting = shown;
    else shown();
  });
  // A file chosen to import says when it was last changed, which dates the document made of it.
  document.addEventListener("change", (event) => {
    const input = event.target;
    const told = input instanceof HTMLInputElement && input.type === "file" && input.form?.elements.namedItem("modified");
    if (!(told instanceof HTMLInputElement)) return;
    const file = input.files?.[0];
    told.value = file ? String(Math.floor(file.lastModified / 1000)) : "";
  }, true);
  // Closed, the menu is its button's again. `toggle` does not bubble.
  document.addEventListener("toggle", (event) => {
    const surface = event.target;
    if (!(surface instanceof Element) || event.newState !== "closed" || !surface.classList.contains(AT)) return;
    surface.classList.remove(AT);
    surface.style.left = surface.style.top = "";
  }, true);
})();
