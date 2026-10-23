// Shared by the landing page and the docs. Change the repo, the release or the donation link here.
const TUMA = {
  repo: "MorrisKM/Tuma",
  version: "1.0.0",
  donate: "https://github.com/sponsors/MorrisKM",
};

(() => {
  const gh = `https://github.com/${TUMA.repo}`;
  const tag = `v${TUMA.version}`;

  document.querySelectorAll("[data-repo]").forEach((a) => (a.href = gh));
  document.querySelectorAll("[data-repo-path]").forEach((a) => (a.href = gh + a.dataset.repoPath));
  document.querySelectorAll("[data-releases]").forEach((a) => (a.href = `${gh}/releases/tag/${tag}`));
  document.querySelectorAll("[data-version]").forEach((e) => (e.textContent = TUMA.version));
  document.querySelectorAll("[data-donate]").forEach((a) => (a.href = TUMA.donate));
  // Installer names as Tauri writes them, for example Tuma_1.0.0_x64-setup.exe.
  document.querySelectorAll("[data-file]").forEach((a) => {
    a.href = `${gh}/releases/download/${tag}/${a.dataset.file.replace("{v}", TUMA.version)}`;
  });

  // Light or dark: the system's choice until the visitor picks one.
  const root = document.documentElement;
  try {
    const saved = localStorage.getItem("tuma-theme");
    if (saved) root.dataset.theme = saved;
  } catch {}
  document.getElementById("theme")?.addEventListener("click", () => {
    const dark = root.dataset.theme
      ? root.dataset.theme === "dark"
      : matchMedia("(prefers-color-scheme: dark)").matches;
    root.dataset.theme = dark ? "light" : "dark";
    try {
      localStorage.setItem("tuma-theme", root.dataset.theme);
    } catch {}
  });

  const nav = document.getElementById("nav");
  const onScroll = () => nav?.classList.toggle("scrolled", scrollY > 8);
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  // Copy buttons copy the text of their code block.
  document.querySelectorAll(".copy").forEach((b) =>
    b.addEventListener("click", async () => {
      const block = b.parentElement.cloneNode(true);
      block.querySelector(".copy").remove();
      const text = b.dataset.copy ?? block.textContent.replace(/^\s*#.*\n|[ \t]+#.*$/gm, "").trim();
      try {
        await navigator.clipboard.writeText(text);
        b.textContent = "Copied";
      } catch {
        b.textContent = "Select and copy";
      }
      setTimeout(() => (b.textContent = "Copy"), 1600);
    }),
  );

  // Fade sections in as they scroll into view.
  const io = new IntersectionObserver(
    (entries) =>
      entries.forEach((e) => {
        if (e.isIntersecting) {
          e.target.classList.add("in");
          io.unobserve(e.target);
        }
      }),
    { threshold: 0.12 },
  );
  document.querySelectorAll(".reveal").forEach((el) => io.observe(el));
})();
