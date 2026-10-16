const paths = {
  grid: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  chat: "M21 11.5a8.5 8.5 0 0 1-8.5 8.5H4l-3 2V11.5A8.5 8.5 0 0 1 9.5 3h3a8.5 8.5 0 0 1 8.5 8.5Z M6 9h10 M6 13h7",
  archive: "M4 4h16v4H4z M6 8v12h12V8 M10 12h4",
  logs: "M5 3h14v18H5z M8 7h8 M8 11h8 M8 15h5",
  settings:
    "M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1Z M15.5 12a3.5 3.5 0 1 1-7 0 3.5 3.5 0 0 1 7 0",
  pulse: "M2 12h4l3-8 5 16 3-8h5",
  play: "m8 5 11 7-11 7Z",
  stop: "M6 6h12v12H6z",
  plus: "M12 5v14 M5 12h14",
  search: "M20 20l-5-5 M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0",
  chevron: "m9 5 7 7-7 7",
  down: "m6 9 6 6 6-6",
  close: "m6 6 12 12 M6 18 18 6",
  download: "M12 3v12 m-5-5 5 5 5-5 M5 16v5h14v-5",
  folder: "M3 6h7l2 2h9v12H3Z",
  users:
    "M16 21v-3a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v3 M13 6a4 4 0 1 1-8 0 4 4 0 0 1 8 0 M18 3a4 4 0 0 1 0 8 M22 21v-3a4 4 0 0 0-3-4",
  radio:
    "M9 8a5 5 0 0 0 0 8 M15 8a5 5 0 0 1 0 8 M5 4a11 11 0 0 0 0 16 M19 4a11 11 0 0 1 0 16 M12 12h.01",
  moon: "M20 14A8 8 0 0 1 10 4a8 8 0 1 0 10 10Z",
  shield: "m12 2 8 4v6c0 5-8 10-8 10S4 17 4 12V6Z m-4 10 3 3 5-6",
  refresh:
    "M20 7v5h-5 M4 17v-5h5 M5 8a8 8 0 0 1 14-3l1 2 M4 17l1 2a8 8 0 0 0 14-3",
  trash: "M4 6h16 M9 6V3h6v3 M6 6l1 15h10l1-15 M10 10v7 M14 10v7",
  clock: "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0 M12 7v5l3 2",
  info: "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0 M12 11v6 M12 7h.01",
  list: "M8 5h13 M8 12h13 M8 19h13 M3 5h.01 M3 12h.01 M3 19h.01",
  sort: "M8 3v18 m-4-4 4 4 4-4 M15 5h6 M15 10h4 M15 15h2",
  check: "m5 12 4 4 10-10",
  key: "M14 10a5 5 0 1 1-10 0 5 5 0 0 1 10 0 M14 10h8 M19 10v4 M22 10v3",
  external: "M14 3h7v7 M21 3 10 14 M10 3H3v18h18v-7",
  network: "M9 3h6v5H9z M3 16h6v5H3z M15 16h6v5h-6z M12 8v4 M6 16v-4h12v4",
  display: "M3 4h18v13H3z M12 17v4 M8 21h8",
};
export const icon = (name, cls = "") =>
  `<svg class="icon ${cls}" viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[name] || paths.info}"/></svg>`;
export const esc = (v) =>
  String(v ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
export const num = (v) => Number(v || 0).toLocaleString("zh-CN");
