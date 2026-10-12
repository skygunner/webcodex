import { useCallback, useEffect, useRef, useState } from "react";
import type { RuntimeV2Client } from "../api/client.js";
import { readTrace, type TracePage, type TraceRequest } from "../api/traces.js";

// Explicit operator reads only: no timer, automatic retry, or background polling.
export function useTraceRead(client: RuntimeV2Client) {
  const current = useRef<AbortController | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const cancel = useCallback(() => {
    current.current?.abort();
    current.current = null;
    setPending(false);
  }, []);
  useEffect(() => () => {
    current.current?.abort();
    current.current = null;
  }, [client]);
  const read = useCallback(async (request: TraceRequest): Promise<TracePage | null | undefined> => {
    current.current?.abort();
    const controller = new AbortController();
    current.current = controller;
    setPending(true);
    setError("");
    try {
      const response = await readTrace(client, request, controller.signal);
      if (controller.signal.aborted || current.current !== controller) return undefined;
      if (!response?.ok || !response.data) {
        setError(response?.status === 403 ? "Administrator diagnostic access required" :
          response?.status === 401 ? "Sign in to inspect diagnostics" :
          response?.data?.error_kind || "Diagnostics unavailable");
        return null;
      }
      return response.data;
    } catch {
      if (controller.signal.aborted || current.current !== controller) return undefined;
      setError("Diagnostics unavailable");
      return null;
    } finally {
      if (current.current === controller) {
        current.current = null;
        setPending(false);
      }
    }
  }, [client]);
  return { read, cancel, pending, error };
}
