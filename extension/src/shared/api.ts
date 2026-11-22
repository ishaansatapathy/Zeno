// The extension's one way to talk to the API.
//
// Types come from the OpenAPI document (api-schema.d.ts), so a route, a field or a status the API no longer
// has is a type error here. The browser sends the person's own Zeno session with every call (the extension is
// allowed to reach the API's address, and nothing else), so there is no password in the extension and nothing
// to sign in to: a person who is signed in to Zeno on the web is signed in here.
//
// A session lasts minutes. A call that answers 401 renews it once and is repeated; many calls failing at once
// renew it once. Every failure comes back in one shape, including a server that cannot be reached.

import createClient from "openapi-fetch";

import type { components, paths } from "./api-schema";

export type Schemas = components["schemas"];

/** What went wrong. `status` is 0 when the server could not be reached at all. */
export interface ApiFailure {
  status: number;
  /** Stable and machine-readable: branch on this, not on the message. */
  code: string;
  message: string;
}

export type Result<T> = { ok: true; data: T } | { ok: false; error: ApiFailure };

/** What the uploader needs from the API, so tests can stand in for it. */
export interface Backend {
  me(): Promise<Result<Schemas["UserResponse"]>>;
  startUpload(input: { id: string; filename: string; title: string; partSize: number }): Promise<Result<Schemas["CreateVideoResponse"]>>;
  partUrl(videoId: string, part: { number: number; size: number; last: boolean }): Promise<Result<Schemas["PartEnvelope"]>>;
  complete(videoId: string, sizeBytes: number): Promise<Result<Schemas["VideoEnvelope"]>>;
  createRecording(input: { id: string; title: string; videoId: string }): Promise<Result<Schemas["RecordingResponse"]>>;
  createLink(recordingId: string): Promise<Result<Schemas["LinkResponse"]>>;
}

type Reply = { data?: unknown; error?: unknown; response: Response };

function failure(reply: Reply): ApiFailure {
  const body = (reply.error ?? {}) as { error?: { code?: string; message?: string } };
  return {
    status: reply.response.status,
    code: body.error?.code ?? "unexpected",
    message: body.error?.message ?? "The server could not do that.",
  };
}

const UNREACHABLE: ApiFailure = { status: 0, code: "unreachable", message: "Zeno could not be reached." };

export class Api implements Backend {
  private readonly client;
  private renewing: Promise<boolean> | null = null;

  constructor(
    baseUrl: string,
    private readonly fetcher: typeof fetch = (...args) => fetch(...args),
  ) {
    this.client = createClient<paths>({
      baseUrl,
      credentials: "include",
      fetch: (request) => this.fetcher(request),
    });
  }

  /** Renews the session: one request however many ask. */
  private renew(): Promise<boolean> {
    this.renewing ??= (async () => {
      try {
        const reply = await this.client.POST("/v1/auth/refresh");
        return reply.response.status === 204;
      } catch {
        return false;
      } finally {
        this.renewing = null;
      }
    })();
    return this.renewing;
  }

  /** Runs a call; if the session has run out, renews it and runs the call once more. */
  private async call<T>(run: () => Promise<Reply>, pick: (data: unknown) => T): Promise<Result<T>> {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      let reply: Reply;
      try {
        reply = await run();
      } catch {
        return { ok: false, error: UNREACHABLE };
      }
      if (reply.response.status === 401 && attempt === 0 && (await this.renew())) continue;
      if (reply.data !== undefined && reply.response.ok) return { ok: true, data: pick(reply.data) };
      return { ok: false, error: failure(reply) };
    }
    return { ok: false, error: UNREACHABLE };
  }

  me() {
    return this.call(
      () => this.client.GET("/v1/auth/me"),
      (data) => (data as Schemas["UserEnvelope"]).user,
    );
  }

  startUpload({ id, filename, title, partSize }: { id: string; filename: string; title: string; partSize: number }) {
    return this.call(
      () => this.client.POST("/v1/videos/streaming", { body: { id, filename, title, content_type: "video/webm", part_size_bytes: partSize } }),
      (data) => data as Schemas["CreateVideoResponse"],
    );
  }

  partUrl(videoId: string, part: { number: number; size: number; last: boolean }) {
    return this.call(
      () => this.client.POST("/v1/videos/{id}/parts", { params: { path: { id: videoId } }, body: part }),
      (data) => data as Schemas["PartEnvelope"],
    );
  }

  complete(videoId: string, sizeBytes: number) {
    return this.call(
      () => this.client.POST("/v1/videos/{id}/complete", { params: { path: { id: videoId } }, body: { size_bytes: sizeBytes } }),
      (data) => data as Schemas["VideoEnvelope"],
    );
  }

  createRecording({ id, title, videoId }: { id: string; title: string; videoId: string }) {
    return this.call(
      () => this.client.POST("/v1/recordings", { body: { id, title, source: "extension", video_id: videoId } }),
      (data) => (data as Schemas["RecordingEnvelope"]).recording,
    );
  }

  createLink(recordingId: string) {
    return this.call(
      () => this.client.POST("/v1/recordings/{id}/links", { params: { path: { id: recordingId } }, body: { id: recordingId } }),
      (data) => (data as Schemas["LinkEnvelope"]).link,
    );
  }
}

/** Whether an error is worth trying again: the network, a busy server, or the limiter, but not a refusal. */
export function transient(error: ApiFailure): boolean {
  return error.status === 0 || error.status === 429 || error.status >= 500;
}
