//! gRPC surface of policy-service.

use afixo_common::{grpc, ids};
use afixo_engine::Sensitivity;
use afixo_proto::{
    CreateRuleRequest, DeleteRuleRequest, DeleteRuleResponse, GetPersonaRequest,
    GetRequesterRequest, ListCandidateRulesRequest, ListCandidateRulesResponse,
    ListPurposesRequest, ListPurposesResponse, ListRulesRequest, ListRulesResponse, Rule,
    auth_service_client::AuthServiceClient, identity_service_client::IdentityServiceClient,
    policy_service_server::PolicyService,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status, transport::Channel};

use crate::repo;

#[derive(Debug, Clone)]
pub struct PolicySvc {
    pool: PgPool,
    identity: IdentityServiceClient<Channel>,
    auth: AuthServiceClient<Channel>,
}

impl PolicySvc {
    pub fn new(pool: PgPool, identity: Channel, auth: Channel) -> Self {
        Self {
            pool,
            identity: IdentityServiceClient::new(identity),
            auth: AuthServiceClient::new(auth),
        }
    }

    async fn require_purpose(&self, name: &str) -> Result<(), Status> {
        match repo::purpose_exists(&self.pool, name).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(Status::invalid_argument("purpose: not in the vocabulary")),
            Err(e) => Err(grpc::db_status("purpose lookup", e)),
        }
    }
}

#[tonic::async_trait]
impl PolicyService for PolicySvc {
    async fn list_purposes(
        &self,
        _req: Request<ListPurposesRequest>,
    ) -> Result<Response<ListPurposesResponse>, Status> {
        let purposes = repo::list_purposes(&self.pool)
            .await
            .map_err(|e| grpc::db_status("list purposes", e))?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(Response::new(ListPurposesResponse { purposes }))
    }

    async fn list_rules(
        &self,
        req: Request<ListRulesRequest>,
    ) -> Result<Response<ListRulesResponse>, Status> {
        let subject_id = ids::parse("subject_id", &req.get_ref().subject_id)?;
        let rules = repo::list_rules(&self.pool, subject_id)
            .await
            .map_err(|e| grpc::db_status("list rules", e))?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(Response::new(ListRulesResponse { rules }))
    }

    async fn create_rule(&self, req: Request<CreateRuleRequest>) -> Result<Response<Rule>, Status> {
        let req = req.into_inner();
        let subject_id = ids::parse("subject_id", &req.subject_id)?;
        let persona_id = ids::parse("persona_id", &req.persona_id)?;
        let requester_id = ids::parse_opt("requester_id", req.requester_id.as_deref())?;
        let purpose = req.purpose.as_deref().filter(|p| !p.is_empty());
        let max_sensitivity = Sensitivity::from_i32_clamped(req.max_sensitivity).as_i16();

        // Validate every reference at write time so an invalid rule is never
        // stored and can never fail open later (FINAL_REPORT §4.4).
        if let Some(p) = purpose {
            self.require_purpose(p).await?;
        }
        self.identity
            .clone()
            .get_persona(GetPersonaRequest {
                persona_id: persona_id.to_string(),
                subject_id: Some(subject_id.to_string()),
            })
            .await
            .map_err(|s| match s.code() {
                tonic::Code::NotFound => {
                    Status::invalid_argument("persona_id: not one of the subject's personas")
                }
                _ => grpc::internal("identity lookup", s),
            })?;
        if let Some(r) = requester_id {
            self.auth
                .clone()
                .get_requester(GetRequesterRequest {
                    requester_id: r.to_string(),
                })
                .await
                .map_err(|s| match s.code() {
                    tonic::Code::NotFound => {
                        Status::invalid_argument("requester_id: unknown requester")
                    }
                    _ => grpc::internal("requester lookup", s),
                })?;
        }

        let allow_keys = req.allow_list.as_ref().map(|a| a.keys.as_slice());
        let row = repo::insert_rule(
            &self.pool,
            repo::NewRule {
                subject_id,
                requester_id,
                purpose,
                persona_id,
                max_sensitivity,
                allow_keys,
                priority: req.priority,
            },
        )
        .await
        .map_err(|e| grpc::db_status("insert rule", e))?;
        tracing::info!(rule_id = %row.id, %subject_id, "rule created");
        Ok(Response::new(row.into()))
    }

    async fn delete_rule(
        &self,
        req: Request<DeleteRuleRequest>,
    ) -> Result<Response<DeleteRuleResponse>, Status> {
        let req = req.get_ref();
        let subject_id = ids::parse("subject_id", &req.subject_id)?;
        let rule_id = ids::parse("rule_id", &req.rule_id)?;
        let deleted = repo::delete_rule(&self.pool, subject_id, rule_id)
            .await
            .map_err(|e| grpc::db_status("delete rule", e))?;
        if deleted {
            Ok(Response::new(DeleteRuleResponse {}))
        } else {
            Err(Status::not_found("rule"))
        }
    }

    async fn list_candidate_rules(
        &self,
        req: Request<ListCandidateRulesRequest>,
    ) -> Result<Response<ListCandidateRulesResponse>, Status> {
        let req = req.get_ref();
        let subject_id = ids::parse("subject_id", &req.subject_id)?;
        let requester_id = ids::parse("requester_id", &req.requester_id)?;
        self.require_purpose(&req.purpose).await?;
        let rules = repo::list_candidates(&self.pool, subject_id, requester_id, &req.purpose)
            .await
            .map_err(|e| grpc::db_status("list candidates", e))?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(Response::new(ListCandidateRulesResponse { rules }))
    }
}
