/* Local only. No network, no storage, no cookies.
   Types the CLI transcript once. The full text stays in the page either way. */
(function () {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  var nodes = document.querySelectorAll("[data-term]");
  for (var n = 0; n < nodes.length; n++) {
    typeOnce(nodes[n]);
  }

  function typeOnce(source) {
    var full = source.textContent || "";
    if (!full) return;
    var box = source.parentElement;
    var reserved = box.offsetHeight;
    var live = document.createElement("pre");
    live.className = "term-live";
    live.setAttribute("aria-hidden", "true");
    source.classList.add("term-source");
    box.style.minHeight = reserved + "px";
    box.appendChild(live);
    var i = 0;
    var step = 4;
    function tick() {
      if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
        live.remove();
        source.classList.remove("term-source");
        return;
      }
      i = Math.min(full.length, i + step);
      live.textContent = full.slice(0, i);
      if (box.scrollHeight > box.clientHeight) box.scrollTop = box.scrollHeight;
      if (i < full.length) window.setTimeout(tick, 14);
    }
    tick();
  }
})();
