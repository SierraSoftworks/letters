// The server's own behaviour on top of yewi's: a right click on a row of a file table, or the menu key
// with the row's name in focus, opens the row's menu where the pointer is. A row with no menu keeps the
// browser's own.
(() => {
  const AT = "lt-context";
  const menuOf = (row) => row.querySelector(".lt-file__more [data-yi-popover] > [popover]");
  document.addEventListener("contextmenu", (event) => {
    const row = event.target instanceof Element && event.target.closest("tr.yi-table__row");
    const surface = row && menuOf(row);
    // A right click in a menu that is open keeps what it shows.
    if (!surface || surface.contains(event.target)) return;
    event.preventDefault();
    if (surface.matches(":popover-open")) surface.hidePopover();
    surface.classList.add(AT);
    surface.showPopover();
    // Kept on the page, however near its edge the pointer is.
    const { width, height } = surface.getBoundingClientRect();
    const fit = (at, size, room) => Math.max(4, Math.min(at, room - size - 4));
    surface.style.left = `${fit(event.clientX, width, innerWidth)}px`;
    surface.style.top = `${fit(event.clientY, height, innerHeight)}px`;
    surface.querySelector("[role^=menuitem]:not([aria-disabled=true])")?.focus();
  });
  // Closed, the menu is its button's again. `toggle` does not bubble.
  document.addEventListener("toggle", (event) => {
    const surface = event.target;
    if (!(surface instanceof Element) || event.newState !== "closed" || !surface.classList.contains(AT)) return;
    surface.classList.remove(AT);
    surface.style.left = surface.style.top = "";
  }, true);
})();
