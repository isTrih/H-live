import test from "node:test";
import assert from "node:assert/strict";
import {createCommunitySettings} from "./community-settings.js";

const save = controller => controller.action({dataset: {action: "community-save"}});
const enable = (controller, checked = true) => controller.change({id: "community-enabled", checked});
const typeKey = (controller, value) => controller.input({id: "community-api-key", value});
function fixture(bridge) {
  const password = {value: "", focus() {}};
  const status = {textContent: ""};
  const root = {innerHTML: "", isConnected: true, querySelector(selector) {
    if (selector === "#community-api-key" && this.innerHTML.includes('id="community-api-key"')) return password;
    if (selector === "#community-settings-status") return status;
    return null;
  }};
  const saved = [], notices = [];
  const controller = createCommunitySettings({bridge, toast: value => notices.push(value), onSaved: value => saved.push(value)});
  return {root, password, status, controller, saved, notices};
}

test("community defaults off and enabling without a key waits for explicit valid save", async () => {
  const calls = [];
  const f = fixture(async method => { calls.push(method); return {}; });
  await f.controller.mount(f.root);
  assert.match(f.root.innerHTML, /id="community-enabled"[^>]*>/);
  assert.doesNotMatch(f.root.innerHTML, /checked|id="community-api-key"/);
  enable(f.controller);
  assert.deepEqual(calls, ["community_settings"]);
  assert.match(f.root.innerHTML, /尚未保存/);
  await save(f.controller);
  assert.deepEqual(calls, ["community_settings"]);
  assert.match(f.root.innerHTML, /请输入社区生态 KEY/);
});

test("an existing stored key or backend development key can enable replay without returning the secret", async () => {
  for (const state of [{key_configured: true}, {development_key_available: true}]) {
    const calls = [];
    const f = fixture(async (method, payload) => {
      calls.push([method, payload]);
      return {...state, enabled: payload?.enabled || false, api_key: "backend-secret-never-public"};
    });
    await f.controller.mount(f.root);
    enable(f.controller);
    if (state.development_key_available) assert.match(f.root.innerHTML, /已检测到本机 KEY/);
    await save(f.controller);
    assert.deepEqual(calls.at(-1), ["community_configure", {enabled: true}]);
    assert.equal(f.saved.length, 1);
    assert.deepEqual(Object.keys(f.saved[0]).sort(), ["development_key_available", "enabled", "key_configured"]);
    assert.doesNotMatch(f.root.innerHTML, /backend-secret-never-public/);
  }
});

test("saving sends a typed key once and clears it before the request finishes", async () => {
  let finish, saves = 0;
  const calls = [];
  const f = fixture((method, payload) => {
    calls.push([method, payload]);
    if (method === "community_settings") return {};
    if (++saves > 1) return {enabled: true, key_configured: true};
    return new Promise(resolve => { finish = resolve; });
  });
  await f.controller.mount(f.root);
  enable(f.controller);
  typeKey(f.controller, "typed-private-key");
  const saving = save(f.controller);
  assert.deepEqual(calls.at(-1), ["community_configure", {enabled: true, api_key: "typed-private-key"}]);
  assert.equal(f.password.value, "");
  assert.doesNotMatch(f.root.innerHTML, /typed-private-key/);
  assert.match(f.root.innerHTML, /id="community-api-key"[^>]*disabled/);
  finish({enabled: true, key_configured: true});
  await saving;
  await save(f.controller);
  assert.deepEqual(calls.at(-1), ["community_configure", {enabled: true}]);
});

test("turning the switch off discards an unsaved key without changing backend state until saved", async () => {
  const calls = [];
  const f = fixture(async (method, payload) => { calls.push([method, payload]); return {enabled: payload?.enabled || false}; });
  await f.controller.mount(f.root);
  enable(f.controller);
  typeKey(f.controller, "discard-this-key");
  enable(f.controller, false);
  assert.deepEqual(calls.map(([method]) => method), ["community_settings"]);
  enable(f.controller);
  await save(f.controller);
  assert.match(f.root.innerHTML, /请输入社区生态 KEY/);
  assert.doesNotMatch(f.root.innerHTML, /discard-this-key/);
  enable(f.controller, false);
  await save(f.controller);
  assert.deepEqual(calls.at(-1), ["community_configure", {enabled: false}]);
});

test("save failures remain in the form without exposing a key echoed by an upstream error", async () => {
  const f = fixture(async method => {
    if (method === "community_settings") return {};
    throw Error("401 Bearer do-not-echo-this-key <script>alert(1)</script>");
  });
  await f.controller.mount(f.root);
  enable(f.controller);
  typeKey(f.controller, "do-not-echo-this-key");
  await save(f.controller);
  assert.match(f.root.innerHTML, /保存失败/);
  assert.match(f.root.innerHTML, /id="community-enabled"[^>]*checked/);
  assert.match(f.root.innerHTML, /id="community-api-key"/);
  assert.doesNotMatch(f.root.innerHTML, /do-not-echo-this-key|Bearer|<script>/);
  assert.equal(f.password.value, "");
  assert.equal(f.saved.length, 0);
});

test("closing settings clears input and a completed in-flight save invalidates replay without repainting", async () => {
  let finish;
  const f = fixture(method => method === "community_settings" ? {} : new Promise(resolve => { finish = resolve; }));
  await f.controller.mount(f.root);
  enable(f.controller);
  typeKey(f.controller, "pending-key");
  f.controller.unmount();
  assert.equal(f.password.value, "");
  await f.controller.mount(f.root);
  enable(f.controller);
  typeKey(f.controller, "second-key");
  const saving = save(f.controller);
  f.controller.unmount();
  f.root.innerHTML = "another page";
  finish({enabled: true, key_configured: true});
  await saving;
  assert.equal(f.root.innerHTML, "another page");
  assert.equal(f.saved.length, 1);
  assert.equal(f.notices.length, 0);
});

test("a failed settings read cannot overwrite an unknown configuration and offers a retry", async () => {
  let attempts = 0;
  const calls = [];
  const f = fixture(async method => {
    calls.push(method);
    if (++attempts === 1) throw Error("private response data");
    return {enabled: true, key_configured: true};
  });
  await f.controller.mount(f.root);
  enable(f.controller);
  await save(f.controller);
  assert.deepEqual(calls, ["community_settings"]);
  assert.match(f.root.innerHTML, /community-reload/);
  assert.doesNotMatch(f.root.innerHTML, /private response data/);
  await f.controller.action({dataset: {action: "community-reload"}});
  assert.deepEqual(calls, ["community_settings", "community_settings"]);
  assert.match(f.root.innerHTML, /id="community-enabled"[^>]*checked/);
});
