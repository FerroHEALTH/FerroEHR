- An AQL comparison between a `DV_DATE_TIME` path that ends on the data value
  itself, such as `c/context/start_time`, and a date-time literal now compares
  the instants both name, so values with different offsets compare correctly.
  The comparison read the whole data value and matched nothing.
