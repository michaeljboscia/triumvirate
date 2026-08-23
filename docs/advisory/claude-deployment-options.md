# Claude Deployment Options: Trust Boundary, Features, and Cost

**Created:** 2026-08-23
**Status:** reference for client engagements. Companion to `docs/pantheon/local-inference-buy-vs-rent.md`, which concluded "rent the intelligence, own the context layer." This document is the detail behind "rent."
**Verified against:** platform.claude.com docs and AWS docs, August 2026. Feature availability moves; re-check before quoting a client.

---

## The five ways to reach the same model

The weights are identical across all of these. What differs is who operates the inference, who can see the traffic, which API features exist, and how it gets billed.

| | Operator | Anthropic sees traffic | Model ID form | Billing |
|---|---|---|---|---|
| **First-party Claude API** | Anthropic | Yes | `claude-opus-5` | Anthropic, direct |
| **Claude Platform on AWS** | Anthropic | Yes | `claude-opus-5` (no prefix) | AWS Marketplace |
| **Amazon Bedrock** | AWS | **No** | `anthropic.claude-opus-5` | AWS native service |
| **Google Vertex AI** | Google | **No** | `claude-opus-5` | GCP native service |
| **Microsoft Foundry** | Microsoft | Varies | see docs | Microsoft Marketplace |

The "Anthropic sees traffic" column is the entire sovereignty conversation. Everything else is procurement and feature negotiation.

---

## What "in your VPC" actually means

It is a term of art and clients will press on it. **The model does not run inside the customer's VPC.** Bedrock and Vertex are managed services; the weights run on AWS or Google infrastructure in their accounts.

Four separate mechanisms do the work:

1. **Private network path.** AWS PrivateLink or GCP Private Service Connect. Traffic goes from the customer subnet to the service endpoint over the provider backbone. Nothing crosses the public internet, and there is no egress to an Anthropic endpoint to explain in a security review.

2. **Anthropic is not in the path.** AWS and Google host the weights themselves. This is the real substance. The sentence that matters to a compliance officer is not "the weights are in our building," it is "our data never leaves our existing cloud vendor relationship."

3. **Compliance inheritance.** The deployment sits under agreements the client already signed: SOC 2, an existing HIPAA BAA, FedRAMP in GovCloud, IRAP. This converts a six-month vendor review into "another service in the account we already audited." It is the entire commercial value of this path.

4. **Region pinning and CMEK.** Residency by region selection, customer-managed keys at rest.

### What it does not do

- **Not air-gappable.** There is a live network path to a managed service. A client with a genuine isolation requirement fails on the first question. That is the residual case where sovereign metal is the only answer.
- **Does not eliminate third-party trust, it relocates it.** The third party moves from Anthropic to AWS. A compliance-driven buyer counts that as a win because they already trust AWS. A control-motivated buyer may not count it at all.

---

## Claude Platform on AWS

Anthropic-operated access to the full Developer Platform through AWS. SigV4 auth, IAM access control, AWS Marketplace billing, bare model IDs. Endpoint pattern `https://aws-external-anthropic.{region}.api.aws/v1/...`. Requires `AWS_REGION` and `ANTHROPIC_AWS_WORKSPACE_ID`; neither has a default and a missing one throws at client construction.

**It solves procurement, not the data path.** Anthropic is the data processor for inference inputs and outputs. The docs go further: *data may not reside in AWS, inference may route to Anthropic's primary cloud, and subservices may change without notice.* Any client who chose AWS for locality reasons has that assumption broken here.

**But residency is purchasable, unlike on Bedrock.** `inference_geo` is supported (it is not on Bedrock or Vertex). `US` pins inference to US data centers at a **1.1x pricing multiplier**; `Global` is standard pricing.

**The trap worth writing down:** the AWS region a workspace binds to does **not** pin inference. Region controls the gateway endpoint and where IAM, CloudTrail, and billing are scoped. Only `inference_geo` controls where the model runs. Getting this wrong in a client document reads as misrepresentation after the fact.

### Billing mechanics

- Denominated in **Claude Consumption Units**, fixed at **$0.01/CCU, never discounted**
- Anthropic rates tokens in USD at standard per-model rates, applies any negotiated discount, then converts to CCUs. A discount means fewer CCUs metered, not a cheaper CCU
- Metered hourly, invoiced monthly **in arrears** on the existing AWS bill, pre-tax
- Not prepaid credits. No balance, no commitment
- **Rates match the first-party Claude API.** AWS states this directly
- **Private offers** must be accepted before the usage occurs. Discounts are never retroactive. An existing Bedrock private offer needs a rep conversation before sign-up

**Unconfirmed:** whether this spend retires against an existing AWS EDP or Private Pricing Agreement. A secondary source claims it does; AWS's own page is silent. For a client with a large AWS commit this is the single most valuable fact in the comparison. Treat it as a question for their AWS rep, not an assertion in a proposal.

### Operational gotchas

- Sign-up **provisions a new Anthropic organization** tied to the AWS account. Separate from any existing org. Keys, workspaces, and Console settings do not carry over
- New orgs land on the **Start tier** with a monthly spend cap. Hitting it fails requests until 00:00 UTC on the first of the next month; retrying sooner does not work. Tier increases go through an account rep, so arrange before production traffic
- Spend is computed at list prices with roughly a **2 hour lag**, so a cap can be overshot and the overshoot is billed
- Separate capacity pool from both first-party and Bedrock. Workloads can run across more than one and fail over

---

## The feature tax on Bedrock and Vertex

Routing through a partner-operated platform costs real API surface. Both drop:

- Message Batches
- Files API
- Models API
- Web fetch
- Code execution
- MCP connector
- Managed Agents
- Programmatic tool calling
- Automatic prompt caching

Bedrock additionally has **no server-side web search**. Vertex has the basic variant only, without dynamic filtering.

**Design consequence:** build retrieval and orchestration so they do not depend on server-side tools. Then the same codebase serves first-party and VPC-routed clients without a fork. If the context layer leans on any of the above, every VPC-routed client becomes a reimplementation project.

---

## Zero Data Retention is not automatically the right goal

ZDR is available on the API and on Claude Platform on AWS by request through an account representative. It sounds like the maximally private choice.

For a regulated client it can be a liability. Firms under recordkeeping obligations (SEC Rule 204-2, FINRA and SEA 17a-4, IRS requirements) must **preserve** records relating to advice, recommendations, and client communications. Chasing ZDR can manufacture a recordkeeping gap.

**The correct architecture:** retention obligations live in the client's systems, not the vendor's. Whatever qualifies as a record goes into their archive and CRM. The vendor's retention setting is not their compliance program and must not be load-bearing for it.

---

## Choosing

| Client situation | Answer |
|---|---|
| Staff want to use AI, nobody is building software | **Seats** (Claude Team or Enterprise), not any API surface |
| Building an app, no unusual data constraints | First-party API |
| Building an app, procurement wants it on the AWS bill | Claude Platform on AWS |
| Compliance requires the model vendor never see the data | Bedrock or Vertex, and accept the feature gaps |
| Genuine air-gap requirement | None of these. This is the only real case for local metal |

Most prospects who say "sovereignty" mean the third row, and a meaningful share of those are satisfied by the second once they understand who is in the path.
