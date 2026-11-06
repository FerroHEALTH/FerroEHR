- A Basic user never carries a `system/` scope, through `ferrohealth-smart`
  0.9.1 (GHSA-jhg2-jwjp-hq74, CWE-269, CWE-863). A `system/` scope granted
  to a Basic user gave that person every resource of a type, a backend
  service's reach. Such a user now refuses the start, naming the user and
  the scope. Upgrade step: give each Basic user the `user/` scopes a person
  holds, and move each machine on Basic to a client of the authorization
  server under the client credentials grant.
