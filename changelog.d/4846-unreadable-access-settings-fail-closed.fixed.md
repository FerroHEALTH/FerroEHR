- An EHR whose `ferroehr.access_control.v1` settings carry the scheme's
  `_type` but do not parse is held closed. Before, such an EHR was served at
  the default level, as if it carried no settings: open to every caller unless
  the server default was restricted, with no privacy ceiling on its
  compositions. Now every request that names the EHR, its reads and its
  export included, is refused with `500`, no query reaches it, and the server
  logs an error naming the EHR.
