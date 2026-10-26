- **The FLAT builder builds an `INTERVAL_EVENT` at an abstract `EVENT` node
  when the input names its width** (#3898), on the 0.0.102 crate line. RM
  data_structures §EVENT makes `EVENT` abstract over `POINT_EVENT` and
  `INTERVAL_EVENT`, and a `/width`, `/math_function` or `|sample_count` under
  such a node (master05 §INTERVAL_EVENT) now chooses the interval; without
  them the node still builds as a `POINT_EVENT`. The flattener writes those
  rows back for an interval event held at an abstract node.
