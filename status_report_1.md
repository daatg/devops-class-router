## Accomplishments:
  Summarize in a few bullet points the progress you've made towards implementing your Continuous Deployment pipeline. Link to 1 non-trivial GitHub commit per person that demonstrate progress toward your accomplishments.
  - Added initial load-balancer setup (in `/router`)
    - Settled barebones tech stack: `actix-web` server + `awc` client for reverse proxy
    - See [0c268386f502a475d05105237d7cebf0200fa22d](https://github.ncsu.edu/hdschnei/CSC-519-project/commit/0c268386f502a475d05105237d7cebf0200fa22d)
  - Added initial setup for linter on Github Actions

## Next Steps:
  Briefly specify the goals for each team member during your next sprint.
  - Goal: implement load balancer functionality with dummy user-agents
  - Goal: finish linting and add `eslint` functionality
    - Add self-hosted runner
  - Goal: Add `cargo test` and `npm test` to actions
## Retrospective for the Sprint: 
  What worked, want didn't work, are what are you going to do differently.
  - Worked well: Choosing a language I already knew for implementing the load balancer. I considered Golang for a brief period but settled on Rust as I could better vet options for implementing a MVP proxy
  - Didn't work well: I wish I had budgeted my time better: between midterms, a short spell of sickness following my flu shot, and other responsibilities, if I had done more work in lecture I would be better off.
  - Do differently: I will block time better next sprint.
