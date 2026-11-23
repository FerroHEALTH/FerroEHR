- A PERSON, ORGANISATION, GROUP, ROLE or AGENT create, update or delete
  settles its access rows in two statements, every delete and retirement
  in one and every insert in the next, where it sent one statement per
  table. A PERSON create, update or delete sends 12 database round trips
  where it sent 20 and about 1 KB less over the database wire, and an
  ORGANISATION, GROUP or ROLE create 10, 10 and 19 where they sent 12, 11
  and 21. Its answers and the rows it leaves stay as they were.
