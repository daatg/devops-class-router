## Accomplishments:
  Summarize in a few bullet points the progress you've made towards implementing your Continuous Deployment pipeline. Link to 1 non-trivial GitHub commit per person that demonstrate progress toward your accomplishments.
  - Finished load-balancer setup (in `/router`)
    - Routes requests to hosts defined in `hosts.yaml`. 
    - Routing times average about ~7ms on barebones requests as per initial localhost testing.
    - Support for timeout handling and configurable timeout penalty in dynamic "moving average" routing.
    - See [2d746910faf3f734f14047886f9978172e657349](https://github.ncsu.edu/hdschnei/CSC-519-project/commit/2d746910faf3f734f14047886f9978172e657349)
## Next Steps:
  Briefly specify the goals for each team member during your next sprint.
  - Goal: finish at least one of the following:
    - Add linting and `eslint` functionality
    - Add self-hosted runner
    - Add `cargo test` and `npm test` to actions
    - Outline deployment playbooks / basic Docker structure
## Retrospective for the Sprint: 
  What worked, want didn't work, are what are you going to do differently.
  - Worked well: Work on the load balancer was relatively easy--progressively implementing functionality while testing on localhost (against a dummy `npm` app from workshop 4) proved to be an effective workflow.
  - Didn't work well: The load balancer work took up all of my time on the project. I did not have time to get to working on Github Actions. That being said, implementing Actions should be a task with much smaller scope than writing the load balancer.
  - Do differently: I have more intentionally scoped goals for this next sprint.
