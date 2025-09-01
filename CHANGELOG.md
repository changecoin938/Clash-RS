# Changelog

All notable changes during local integration are documented here.

## Unreleased

- fix(build): restore stub transport modules `h3` and `xHTTP` to resolve missing-module errors (E0583) and keep green builds while full implementations are prepared
- fix(converters): disable `reality` mapping in VLESS/Trojan converters to avoid unresolved imports while the transport is not present in this build
- docs(ffi): confirm iOS XCFramework builds with `ring` crypto backend; leave `aws-lc-rs` for non-Apple targets
- docs(run): add smoke-test steps for HTTP/SOCKS/Mixed and API endpoints

Notes:
- The restored `h3` and `xHTTP` transports are minimal pass-through stubs in this build. Full behavior can be re-enabled behind feature flags in a follow-up.
- REALITY transport is intentionally disabled in converters until its module is reintroduced end-to-end.
