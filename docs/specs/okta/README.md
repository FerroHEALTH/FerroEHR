# Okta Managment API OpenAPI Specification

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Support](https://img.shields.io/badge/support-Developer%20Forum-blue.svg)](https://devforum.okta.com/)
[![npm version](https://img.shields.io/npm/v/@okta/openapi.svg?style=flat-square)](https://www.npmjs.com/package/@okta/okta)

## Overview

This repository contains:

- The OpenAPI specification that we use to build our Management SDKs
- Directory [dist/current/](dist/current/) has the current version of our OpenAPI v3 specs for the Okta Managment API
- Each historical Okta release in format [`YYYY.MM.POINT`](https://help.okta.com/en-us/content/topics/releasenotes/production.htm) is retained in the [dist/](dist/) directory

Note: this is a snapshot of the OpenAPI spec generated directly from the Okta
Management API. This repository is not intended to take pull requests from the
community. Do report issues here in Github so others in the community may
benefit from the feedback. However, we encourage opening a support ticket with
[Okta Support](https://support.okta.com/) to get help directly.

## Historical

The original Swagger specs developed by the Okta Developer Experience team live
on the <a href="../../tree/legacy-v1-swagger">tree/legacy-v1-swagger</a> branch. The
last artifacts from that branch are persisted in the
[dist/legacy-v1-swagger](dist/legacy-v1-swagger/) directory for the sake of
convenience.

## NPM Artifact

A published NPM artifact would be available to the public as
[@okta/openapi](https://www.npmjs.com/package/@okta/openapi).

## Building An SDK

All of our management SDKs must be built from this spec.

## Issues

If you notice any issues with the spec files while working with any of our
management SDKs, please open a Github issue in the corresponding management SDK
repo, it would be triaged from there.
