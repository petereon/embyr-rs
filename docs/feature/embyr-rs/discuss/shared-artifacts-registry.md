# Shared Artifacts Registry — embyr-rs

> Feature: embyr-rs full implementation
> Wave: DISCUSS / Phase 2

All artifacts passed between journey steps across all personas.

| Artifact | Type | Produced by | Consumed by | Source of Truth |
|---|---|---|---|---|
| `${embyr_grpc_endpoint}` | host:port | Operator config | SDK (gRPC/gRPC-Web) | service config `server.grpc_port` |
| `${embyr_rest_endpoint}` | host:port | Operator config | SDK (REST/BrowserChannel/gRPC-Web) | service config `server.rest_port` |
| `${embyr_admin_endpoint}` | host:port | Operator config | Admin API callers | service config `admin.port` |
| `${admin_key}` | string | Operator config | Admin API `Authorization: Bearer` | service config `admin.key` |
| `${project_id}` | string | Admin Create Project | SDK config, all resource names | projects table PK |
| `${auth_key}` | string | Admin Create Project (one-time) | SDK `Authorization: Bearer`, ECIES derive | never stored in plaintext |
| `${document_path}` | resource name | Server (GetDocument / CreateDocument) | SDK read/listen operations | documents.path |
| `${resume_token}` | bytes | Server (Listen targetChange CURRENT) | SDK reconnect (addTarget.resumeToken) | decoded: RFC3339Nano timestamp |
| `${transaction_id}` | bytes | Server (BeginTransaction) | SDK (GetDocument, Commit, Rollback) | transactions.id |
| `${stream_id}` | string | Server (Write handshake) | SDK (Write loop WriteRequest) | 16-hex unix nanoseconds |
| `${stream_token}` | string | Server (Write handshake + each response) | SDK (Write loop WriteRequest) | RFC3339Nano timestamp |
| `${browser_channel_sid}` | string | Server (new-session POST) | SDK (all subsequent BC requests) | 24-hex random |
| `${agent_endpoint}` | host:port | Customer K8s deployment | Admin Create Project, embyr SaaS | K8s service DNS |
| `${agent_ca_pem}` | PEM bytes | Customer PKI | Admin Create Project (`backend_agent_ca`) | customer certificate authority |
| `${embyr_agent_ca_pem}` | PEM bytes | embyr operator PKI | Agent deploy (`EMBYR_AGENT_CA`) | embyr operator cert |
| `${secret_arn}` | AWS ARN | Customer AWS setup | Admin Create Project (`backend_secret_arn`) | AWS Secrets Manager |
| `${secret_gcp_name}` | GCP resource name | Customer GCP setup | Admin Create Project (`backend_secret_gcp`) | GCP Secret Manager |
| `${commit_time}` | timestamp | Server (Commit / Write stream) | SDK (WriteResult.update_time) | server clock at commit |
| `${read_time}` | timestamp | Server (GetDocument / RunQuery) | SDK (Document.read_time) | server clock at read |
