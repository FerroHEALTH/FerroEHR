- A party delete (`DELETE /demographic/{agent,group,organisation,person,role}/{uid_based_id}`)
  and the party relationship delete now answer `400` when the path names the
  versioned object instead of its latest `OBJECT_VERSION_ID`, and delete
  nothing. They answered `204` and deleted the current version. The release
  requires the `preceding_version_uid` in that path (ITS-REST
  `agent_delete.yaml` and its four siblings) (#4478).
