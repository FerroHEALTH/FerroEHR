- The access log's readers plan the entries past the search index's cursor
  as the narrow key range they are. A sink's next access-log entry is read
  through the key range of its window, and the open window is bounded by the
  newest entry, so neither the delivery read nor the person's access trail
  starts parallel workers or sorts that window. On 200,000 entries the
  delivery read touches about 300 pages instead of about 646,000, and takes
  about 1 ms instead of 150 to 210 ms.
