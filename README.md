# DevOps Final Project
### "Ensuring Quality of Service"
Author: Quaker Schneider (hdschnei@ncsu.edu)

# Addendum for Final Submission

`/router/` - contains the Rust load-balancer application and associated files.

`/coffee-project/` - contains the Coffee web app and Docker containerization.

`/.github/workflows/` contains the github Actions workflows, which require the setup of self-hosted runners. I could not get my VCL instances to run these properly.

Ansible playbooks are contained top-level, and split into app deployment and router deployment. `deploy-all.yaml` does what it says on the tin.

To run `deploy-all.yaml`, there is a Dockerfile top-level that can be built via `docker build . --tag deploy`. To run it, run `docker run -v {PATH TO YOUR SSH FOLDER}:/root/.ssh deploy`.

Lastly, to test the router locally, enter the `/router/` subdirectory and run `RUST_LOG=info cargo run`.

# Problem Statement

Frequently, web applications will be in the business of serving a set of users. (Shocking, I know.)

Load might come intermittently for a client using our API. An e-commerce shop on our platform might gain sudden viral popularity. A user on our site might be making a big batch of changes. Whatever the case is, we are faced with a problem: how do we balance the needs of that high-demand user against the usability of the application as a whole?

One approach is to do nothing. How this will play out depends on your server architecture. If you have one host, it will likely slow down dramatically. If you have a load balancer, the effects of this traffic uptick will depend on what balancing algorithm you use.

Load balancers usually try their best to evenly distribute traffic, but this doesn't do a good job of balancing the needs of all users. If one overloads the system, everyone pays the price.

Thus comes my pipeline as a solution to this issue. Central to it is the custom load balancer implementation, which enables both continuous deployment and load isolation techniques in order to ensure that quality of service is maintained at all times for the end users of our application.

# Pipeline design
### System Overview
![DEVOPS 519 Project - HTTP Setup](https://github.com/user-attachments/assets/1a9a2a14-ec6e-4779-8935-fe11a68fe0d8)

### Deployment Architecture
![DEVOPS 519 Project - Devops Flow](https://github.com/user-attachments/assets/1ab7792a-d90b-427d-97e2-b3bfd48a2802)

## Continuous (Piecewise) Deployment
Continuous deployment is handled through partitioning hosts and installing new releases on half of the host space at a time. Quality may degrade for a few minutes across the whole system, but that is a signficant improvement over having system downtime. Especially if these updates occur during periods of low load, this quality may not degrade noticeably.

![DEVOPS 519 Project - Devops flow expanded](https://github.com/user-attachments/assets/389b2e48-264d-4538-b104-ba5e743c6d57)

## Load Isolation
Through combining a twofold approach to load balancing, we can partition requests into static, hashed routing (where the same agent's requests always end up at the same host) and dynamic, least-latency routing (which evenly distributes load across hosts). This swapover triggers at a cutoff point, which can be scaled in production should testing indicate a different coefficient is approprate.  

[loadbalancer.webm](https://github.com/user-attachments/assets/0517c18c-d2b8-49e3-a592-a7eb8ecf3f53)

# Use Case: Code Deployment
```
1 Preconditions
   Devops Infrarstructure provisioned and deployed.
   Application hosts provisioned and deployed.
2 Main Flow
   Developer will initiate the pipeline by making sequential PRs--first to main[S1], and then to a release branch with tag[S3].  
   Tests and linting run on the initial PR to main[S2]. Release is deployed in a half-half structure to the application hosts. 
   Monitoring of latency post-deployment is available[S4].
3 Subflows
  [S1] User provides PR message and requests appropriate reviewers.
  [S2] GitHub actions creates test environment and tests code, in parallel with a linter.
  [S3] User provides PR message and requests appropriate reviewers (including the Release Engineer).
  [S4] The load balancer will provide a secure endpoint for monitoring of response times and host health.
4 Alternative Flows
  [E1] Test suite fails in PR to main.
  [E2] Linting fails in PR to main.
  [E3] Release-deployed hosts are unresponsive.
```
