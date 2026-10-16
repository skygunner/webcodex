"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { fetchToFile, resolveNpmNetworkOptions } = require("../install");

const PROXY_URL = "http://127.0.0.1:9/";

test("IPv6 NO_PROXY bypasses proxy for literal hosts and respects port matching", () => {
  const target = "http://[::1]:4873/archive.tgz";
  const env = (rule) => ({ npm_config_proxy: PROXY_URL, NO_PROXY: rule });
  for (const rule of ["[::1]", "[::1]:4873", "::1"]) {
    assert.equal(resolveNpmNetworkOptions(target, env(rule)).proxy, null, rule);
  }
  for (const rule of ["[::2]", "[::1]:9999"]) {
    assert.equal(resolveNpmNetworkOptions(target, env(rule)).proxy.href, PROXY_URL, rule);
  }
  assert.equal(
    resolveNpmNetworkOptions("https://[2001:db8::10]/artifact", env("[2001:db8::10]")).proxy,
    null
  );
});

test("IPv6 NO_PROXY canonicalizes equivalent addresses and rejects malformed bracket suffixes", () => {
  const target = "http://[::1]:4873/archive.tgz";
  const env = (rule) => ({ npm_config_proxy: PROXY_URL, NO_PROXY: rule });
  for (const rule of [
    "[0:0:0:0:0:0:0:1]",
    "0:0:0:0:0:0:0:1",
    "[0:0:0:0:0:0:0:1]:4873"
  ]) {
    assert.equal(resolveNpmNetworkOptions(target, env(rule)).proxy, null, rule);
  }
  for (const rule of ["[::1]garbage", "[::1]:", "[::1]:bogus", "[0:0:0:0:0:0:0:1]:9999"]) {
    assert.equal(resolveNpmNetworkOptions(target, env(rule)).proxy.href, PROXY_URL, rule);
  }
});

async function ipv6Server(t, handler) {
  const server = http.createServer(handler);
  try {
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "::1", resolve);
    });
  } catch (error) {
    if (server.listening) server.close();
    if (["EAFNOSUPPORT", "EADDRNOTAVAIL", "EPROTONOSUPPORT"].includes(error.code)) {
      t.skip("IPv6 loopback is unavailable on this test runner");
      return null;
    }
    throw error;
  }
  return server;
}

test("npm downloader uses an HTTP proxy listening on a bracketed IPv6 address", async (t) => {
  const proxy = await ipv6Server(t, (request, response) => {
    assert.equal(request.url, "http://origin.invalid/artifact.tgz");
    response.end("proxied-ipv6");
  });
  if (!proxy) return;
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "webcodex-ipv6-proxy-"));
  const output = path.join(temp, "download.bin");
  try {
    await fetchToFile("http://origin.invalid/artifact.tgz", output, {
      label: "IPv6 proxy test",
      firstByteTimeoutMs: 2000,
      inactivityTimeoutMs: 2000,
      totalTimeoutMs: 4000,
      maxBytes: 1024,
      environment: { npm_config_proxy: "http://[::1]:" + proxy.address().port }
    });
    assert.equal(fs.readFileSync(output, "utf8"), "proxied-ipv6");
  } finally {
    await new Promise((resolve) => proxy.close(resolve));
    fs.rmSync(temp, { recursive: true, force: true });
  }
});

test("npm downloader honors bracketed IPv6 NO_PROXY instead of using an unrelated proxy", async (t) => {
  const origin = await ipv6Server(t, (_request, response) => response.end("direct-ipv6"));
  if (!origin) return;
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "webcodex-ipv6-bypass-"));
  const output = path.join(temp, "download.bin");
  try {
    await fetchToFile("http://[::1]:" + origin.address().port + "/artifact.tgz", output, {
      label: "IPv6 NO_PROXY test",
      firstByteTimeoutMs: 2000,
      inactivityTimeoutMs: 2000,
      totalTimeoutMs: 4000,
      maxBytes: 1024,
      environment: { npm_config_proxy: PROXY_URL, NO_PROXY: "[::1]" }
    });
    assert.equal(fs.readFileSync(output, "utf8"), "direct-ipv6");
  } finally {
    await new Promise((resolve) => origin.close(resolve));
    fs.rmSync(temp, { recursive: true, force: true });
  }
});
