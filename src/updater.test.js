import test from "node:test";
import assert from "node:assert/strict";
import { createUpdaterController } from "./updater.js";

function fixture({ guard, prepare, download, install, check } = {}) {
  const calls = [];
  const update = { version: "0.1.1", body: "修复日志", date: "2026-10-14T00:00:00Z",
    close: async () => calls.push("close"),
    download: async (progress) => { calls.push("download"); await download?.(progress); },
    install: async () => { calls.push("install"); await install?.(); } };
  const controller = createUpdaterController({
    checkUpdate: async () => { calls.push("check"); return check ? check() : update; },
    call: async (name) => {
      calls.push(name);
      if (name === "update_install_guard") return guard ? guard() : { ready: true };
      if (name === "prepare_update_install") return prepare ? prepare() : { ready: true };
      return {};
    },
  });
  return { controller, calls, update };
}

test("check never downloads or installs and broadcasts release notes", async () => {
  const { controller, calls } = fixture();
  const seen = [];
  const unsubscribe = controller.subscribe((s) => seen.push(s.status));
  await controller.checkForUpdates();
  assert.deepEqual(calls, ["check"]);
  assert.equal(controller.getState().body, "修复日志");
  assert.deepEqual(seen, ["idle", "checking", "available"]);
  unsubscribe();
});

test("confirmed update downloads before pausing active tasks", async () => {
  const {controller,calls}=fixture({guard:()=>({ready:false,reason:"monitoring"})});
  await controller.checkForUpdates();
  assert.equal(await controller.installUpdate(),true);
  assert.ok(calls.indexOf("download") < calls.indexOf("prepare_update_install"));
  assert.equal(calls.at(-1),"install");
});

test("download then second guard prevents a new task from being interrupted", async () => {
  let busy = true;
  const { controller, calls } = fixture({ prepare: () => ({ ready: !busy, reason: "exporting" }) });
  await controller.checkForUpdates();
  assert.equal(await controller.installUpdate(), false);
  assert.equal(calls.includes("install"), false);
  busy = false;
  await controller.installUpdate();
  assert.equal(calls.filter((c) => c === "download").length, 1);
  assert.deepEqual(calls.slice(-3), ["update_install_guard", "prepare_update_install", "install"]);
});

test("signature failure cannot shut down engine or install and is safely explained", async () => {
  const { controller, calls } = fixture({ download: () => { throw Error("signature mismatch https://private:secret@host"); } });
  await controller.checkForUpdates();
  await controller.installUpdate();
  assert.equal(calls.includes("prepare_update_install"), false);
  assert.equal(calls.includes("install"), false);
  assert.match(controller.getState().message, /签名验证失败/);
  assert.doesNotMatch(controller.getState().message, /private|secret/);
});

test("installer launch failure resumes normal backend requests", async () => {
  const { controller, calls } = fixture({ install: () => { throw Error("private installer failure"); } });
  await controller.checkForUpdates();
  await controller.installUpdate();
  assert.deepEqual(calls.slice(-3), ["prepare_update_install", "install", "cancel_update_install"]);
  assert.equal(controller.getState().busy, false);
  assert.equal(controller.getState().canInstall, true);
});

test("repeated clicks cannot overlap download or install", async () => {
  let finish;
  const waiting = new Promise((resolve) => { finish = resolve; });
  const { controller, calls } = fixture({ download: () => waiting });
  await controller.checkForUpdates();
  const first = controller.installUpdate();
  assert.equal(await controller.installUpdate(), false);
  await controller.checkForUpdates();
  finish();
  await first;
  assert.equal(calls.filter((c) => c === "check").length, 1);
  assert.equal(calls.filter((c) => c === "download").length, 1);
});

test("unknown content length uses indeterminate progress and Finished does not install early", async () => {
  let controller;
  const result = fixture({ download: async (progress) => {
    progress({ event: "Started", data: {} });
    progress({ event: "Progress", data: { chunkLength: 4096 } });
    assert.equal(controller.getState().progress, null);
    progress({ event: "Finished" });
    assert.equal(result.calls.includes("prepare_update_install"), false);
  } });
  controller = result.controller;
  await controller.checkForUpdates();
  await controller.installUpdate();
  assert.equal(controller.getState().progress, 100);
});

test("check closes the prior update resource and safe errors never echo backend text", async () => {
  let first = true;
  const f = fixture();
  const controller = createUpdaterController({ checkUpdate: async () => {
    if (first) { first = false; return f.update; }
    throw Error("https://user:password@private/update.json");
  } });
  await controller.checkForUpdates();
  await controller.checkForUpdates();
  assert.deepEqual(f.calls, ["close"]);
  assert.equal(controller.getState().canInstall, false);
  assert.doesNotMatch(controller.getState().message, /password|private/);
});
