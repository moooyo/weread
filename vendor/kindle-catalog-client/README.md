# Fixed WeRead catalog client candidate

This component prepares the client side of the coordinated shared catalog runtime
extension. It is not enabled, installed, or part of the ordinary development
archive. The [integration record](../../docs/CATALOG_ENTRY_ADAPTER.md)
tracks the agreed WeRead interface and pending shared-build and device gates.

`catalog-client.c` is pinned to Aidoku revision
`78c5b2bc09a469133c886eb0d73aaf9ed63195b2` without functional edits. The local
`common.h` binds it to `/mnt/us/documents/weread.sh` and omits the unused Aidoku
dispatcher definitions. The host protocol fixture is adapted from the same
revision with a WeRead fixture name and source path. Additional cases exercise
the actual embedded-cover document size and the existing file-size budget.
Host protocol tests require a C compiler and `libcjson.so.1` on `test-env`;
the test session-token provider and HTTP catalog service are private fixtures.

The client uses the native loopback catalog service and firmware libraries in
place. It admits a single fixed document and verifies its MIME, size, timestamp,
catalog identity and acknowledged readback. A quiet update records launch-time
recent access; a later normal notification preserves that timestamp. Its
whole-process alarm and transaction/output budgets remain intact. Credentials
and raw catalog identifiers are not emitted. No firmware library is bundled.

`scripts/build-native-catalog-client.sh` builds a separate ARM hard-float glibc
process using the existing native-helper toolchain. The Rust application remains
unchanged and static musl. An agreed shared runtime must validate and retain the
result as an immutable versioned helper before the launcher can consume it.
This preparation neither selects a project license nor declares new device
compatibility or a measured performance improvement.
