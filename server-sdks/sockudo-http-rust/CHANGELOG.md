# Changelog

## 2.2.0 - 2026-08-17

- Added `force_reconnect_user` for `POST /users/{userId}/force_reconnect`.
- Corrected webhook signature header names to match the server and fixed the README's public type
  and module names.
- Reduced avoidable clones in request construction.

## 2.1.0 - 2026-06-27

- Preserved future webhook event names, fields, and nested payload values.
