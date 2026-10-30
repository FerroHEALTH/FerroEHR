- FerroEHR runs on `ferrohealth-server` 0.4.1, which keeps a product's own
  fallback and the layers around it when the stack merges its routes
  (FerroHEALTH/FerroSYS#199). FerroEHR, v5.6.0-rc3 included, was not exposed
  to the regression 0.4.1 fixes: the REST API and the viewer declare their
  own fallback inside their layers, and the management endpoints guard each
  route they serve, so a path no route matches answers as it did before.
