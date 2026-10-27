/** A value offered in a header's name or value cell. */
export interface Choice {
  value: string;
  detail?: string;
}

/** Request headers worth offering, most used first. */
export const HEADER_NAMES: Choice[] = [
  { value: "Content-Type", detail: "Type of the body" },
  { value: "Accept", detail: "Types the response may have" },
  { value: "Authorization", detail: "Credentials" },
  { value: "Cache-Control", detail: "Caching directives" },
  { value: "Cookie", detail: "Cookies to send" },
  { value: "User-Agent", detail: "Who is asking" },
  { value: "Accept-Encoding", detail: "Compression the response may use" },
  { value: "Accept-Language", detail: "Preferred languages" },
  { value: "Origin", detail: "Where a cross-origin request comes from" },
  { value: "Referer", detail: "The page that linked here" },
  { value: "If-None-Match", detail: "Only if the ETag changed" },
  { value: "If-Match", detail: "Only if the ETag still matches" },
  { value: "If-Modified-Since", detail: "Only if changed since a date" },
  { value: "X-Api-Key", detail: "API key, a common convention" },
  { value: "X-Request-Id", detail: "Id to trace the request" },
  { value: "X-Correlation-Id", detail: "Id to trace a chain of requests" },
  { value: "X-Forwarded-For", detail: "Client address behind a proxy" },
  { value: "X-Forwarded-Proto", detail: "Original protocol behind a proxy" },
  { value: "X-Requested-With", detail: "XMLHttpRequest, for some servers" },
  { value: "Idempotency-Key", detail: "Makes a retried POST safe" },
  { value: "Content-Encoding", detail: "Compression of the body" },
  { value: "Content-Language", detail: "Language of the body" },
  { value: "Content-Disposition", detail: "File name of the body" },
  { value: "Accept-Charset", detail: "Character sets the response may use" },
  { value: "Connection", detail: "keep-alive or close" },
  { value: "Range", detail: "Part of the resource" },
  { value: "Pragma", detail: "Legacy caching" },
  { value: "Host", detail: "Usually set from the URL" },
  { value: "Prefer", detail: "Preferences, such as return=minimal" },
  { value: "DNT", detail: "Do not track" },
  { value: "Upgrade-Insecure-Requests", detail: "Prefers HTTPS" },
  { value: "Access-Control-Request-Method", detail: "For a preflight request" },
  { value: "Access-Control-Request-Headers", detail: "For a preflight request" },
];

const TYPES: Choice[] = [
  { value: "application/json" },
  { value: "application/x-www-form-urlencoded" },
  { value: "multipart/form-data" },
  { value: "text/plain" },
  { value: "text/html" },
  { value: "application/xml" },
  { value: "text/xml" },
  { value: "application/graphql" },
  { value: "application/octet-stream" },
  { value: "application/ld+json" },
  { value: "application/vnd.api+json" },
  { value: "application/merge-patch+json" },
  { value: "application/json-patch+json" },
];

/** Values worth offering, by header name in lower case. */
const VALUES: Record<string, Choice[]> = {
  "content-type": TYPES,
  accept: [{ value: "*/*", detail: "Anything" }, ...TYPES, { value: "text/event-stream", detail: "Server-sent events" }],
  authorization: [
    { value: "Bearer {{token}}", detail: "A token" },
    { value: "Basic ", detail: "base64 of user:password" },
    { value: "ApiKey ", detail: "A key" },
    { value: "Digest ", detail: "Digest auth" },
  ],
  "cache-control": [
    { value: "no-cache", detail: "Revalidate first" },
    { value: "no-store", detail: "Never cache" },
    { value: "max-age=0" },
    { value: "max-age=3600" },
    { value: "only-if-cached" },
  ],
  "accept-encoding": [{ value: "gzip, deflate, br" }, { value: "gzip" }, { value: "br" }, { value: "identity", detail: "Uncompressed" }],
  "accept-language": [{ value: "en-US,en;q=0.9" }, { value: "en" }, { value: "*" }],
  "accept-charset": [{ value: "utf-8" }],
  "content-encoding": [{ value: "gzip" }, { value: "br" }, { value: "deflate" }],
  connection: [{ value: "keep-alive" }, { value: "close" }],
  "x-requested-with": [{ value: "XMLHttpRequest" }],
  "x-request-id": [{ value: "{{$guid}}", detail: "A new id each send" }],
  "x-correlation-id": [{ value: "{{$guid}}", detail: "A new id each send" }],
  "idempotency-key": [{ value: "{{$guid}}", detail: "A new id each send" }],
  "x-forwarded-proto": [{ value: "https" }, { value: "http" }],
  prefer: [{ value: "return=minimal" }, { value: "return=representation" }, { value: "respond-async" }],
  pragma: [{ value: "no-cache" }],
  dnt: [{ value: "1" }],
  "upgrade-insecure-requests": [{ value: "1" }],
  range: [{ value: "bytes=0-1023", detail: "The first KB" }],
  "access-control-request-method": ["GET", "POST", "PUT", "PATCH", "DELETE"].map((value) => ({ value })),
  "content-disposition": [{ value: 'attachment; filename="file.txt"' }, { value: "inline" }],
};

/** Values to offer for a header, by its name in any case. */
export function headerValues(name: string): Choice[] {
  return VALUES[name.trim().toLowerCase()] ?? [];
}
