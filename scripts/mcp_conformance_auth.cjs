// The pinned referee has no authorization-header option; scope injection to the fixture URL.
const originalFetch = globalThis.fetch;
const fixtureUrl = process.env.WEBCODEX_MCP_CONFORMANCE_FIXTURE_URL;
const token = process.env.WEBCODEX_MCP_CONFORMANCE_TOKEN;
if (!fixtureUrl || !token) throw new Error('missing conformance fixture authentication');

globalThis.fetch = (input, init) => {
  const request = new Request(input, init);
  if (request.url === fixtureUrl) request.headers.set('Authorization', `Bearer ${token}`);
  return originalFetch(request);
};
