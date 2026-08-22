# afixo-proto

The wire contract, generated from `proto/afixo/v1/*.proto` by
`tonic-prost-build` in `build.rs` (needs `protoc` on PATH). `src/lib.rs` is the
only hand-written file: it includes the generated module and exports service
names.

```
common.proto      Sensitivity, Subject, Field, Persona, Requester, Purpose, AllowList, Rule
auth.proto        AuthService       identity.proto   IdentityService
policy.proto      PolicyService     disclosure.proto DisclosureService
audit.proto       AuditService (Record · ListDecisions · VerifyChain) + DisclosureDecided
```

## Rules

- Change the `.proto` first, rebuild, then fix the compile errors in services.
  Never edit generated code (there is none checked in).
- Proto3 `optional` is used where "unset" is meaningful (`requester_id`,
  `purpose` on a rule). Message-typed fields (`AllowList`, `Timestamp`) carry
  presence natively.
- Field numbers are forever: never renumber or reuse; mark removed fields
  `reserved`.
- Comments in the `.proto` files are the API documentation for RPC semantics
  (status codes, validation); keep them current.

Generated types are re-exported at the crate root (`afixo_proto::Rule`,
`afixo_proto::policy_service_client::PolicyServiceClient`, …).
`names::*` hold the package-qualified service names.
