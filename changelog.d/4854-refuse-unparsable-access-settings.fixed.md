- A CONTRIBUTION or an EHR Extract import that carries
  `ferroehr.access_control.v1` settings which do not parse is refused with
  `400`, naming the first setting that does not parse. Before, such settings
  were stored, and since #4846 they hold the EHR closed, with no way to repair
  it over REST.
