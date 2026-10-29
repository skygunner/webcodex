// The pinned referee has no authorization-header option; scope injection to the fixture URL.
const originalFetch = globalThis.fetch;
const fixtureUrl = process.env.WEBCODEX_MCP_CONFORMANCE_FIXTURE_URL;
const token = process.env.WEBCODEX_MCP_CONFORMANCE_TOKEN;
const harnessDir = process.env.WEBCODEX_MCP_CONFORMANCE_HARNESS_BUILD;
if (!fixtureUrl || !token || !harnessDir) throw new Error('missing conformance fixture authentication');
const { createRequire } = require('node:module');
const { resolve } = require('node:path');
const undici = createRequire(resolve(harnessDir, 'package.json'))('undici');
const fixture = new URL(fixtureUrl);
const loopback = new Set(['127.0.0.1', 'localhost', '[::1]']);
if (fixture.protocol !== 'http:' || !loopback.has(fixture.hostname) || !fixture.port || fixture.username || fixture.password || fixture.search || fixture.hash) {
  throw new Error('invalid conformance fixture URL');
}
function isFixture(url) {
  const target = new URL(url);
  return target.protocol === fixture.protocol && loopback.has(target.hostname) &&
    target.port === fixture.port && target.pathname === fixture.pathname &&
    !target.username && !target.password && !target.search && !target.hash;
}
const originalRequest = undici.request;
undici.request = (url, options) => {
  if (isFixture(url)) {
    options = { ...options, headers: { ...options.headers, authorization: `Bearer ${token}` } };
  }
  return originalRequest(url, options);
};

globalThis.fetch = (input, init) => {
  const request = new Request(input, init);
  if (isFixture(request.url)) request.headers.set('Authorization', `Bearer ${token}`);
  return originalFetch(request);
};
