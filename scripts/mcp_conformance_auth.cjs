// The pinned referee has no authorization-header option; scope injection to the fixture URL.
const originalFetch = globalThis.fetch;
const fixtureUrl = process.env.WEBCODEX_MCP_CONFORMANCE_FIXTURE_URL;
const token = process.env.WEBCODEX_MCP_CONFORMANCE_TOKEN;
const harnessDir = process.env.WEBCODEX_MCP_CONFORMANCE_HARNESS_BUILD;
if (!fixtureUrl || !token || !harnessDir) throw new Error('missing conformance fixture authentication');
const { createRequire } = require('node:module');
const undici = createRequire(`${harnessDir}/package.json`)('undici');
const originalRequest = undici.request;
undici.request = (url, options) => {
  if (String(url) === fixtureUrl) {
    options = { ...options, headers: { ...options.headers, authorization: `Bearer ${token}` } };
  }
  return originalRequest(url, options);
};

globalThis.fetch = (input, init) => {
  const request = new Request(input, init);
  if (request.url === fixtureUrl) request.headers.set('Authorization', `Bearer ${token}`);
  return originalFetch(request);
};
