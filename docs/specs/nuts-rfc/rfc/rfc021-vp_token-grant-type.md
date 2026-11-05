# RFC021 VP Token Grant Type

|                           |                  |
|:--------------------------|:-----------------|
| Nuts foundation           | W.M. Slakhorst   |
| Request for Comments: 021 | Nedap            |
|                           | September 2023   |

## VP Token Grant Type

### Abstract

This specification defines the use of a Verifiable Presentation Bearer Token as a means for requesting an OAuth 2.0 access token.

### Status of document

This document is currently in draft.

### Copyright Notice

![](../.gitbook/assets/license.png)

This document is released under the [Attribution-ShareAlike 4.0 International \(CC BY-SA 4.0\) license](https://creativecommons.org/licenses/by-sa/4.0/).

## 1.  Introduction

A Verifiable Presentation (VP) [VP] is an encoding that enables identity and security information based on Verifiable Credentials (VC) to be shared across security domains. 
A security token is generally issued by an Authorization Server and consumed by a client that relies on its content to make authorization decisions.
A Verifiable Presentation can be used as a security token. When used as a security token, the wallet of the holder assumes the role of the Authorization Server.

The OAuth 2.0 Authorization Framework [RFC6749] provides a method for making authenticated HTTP requests to a resource using an access token. 
Access tokens are issued to third-party clients by an Authorization Server (AS) with the (sometimes implicit) approval of the resource owner. 
In OAuth, an authorization grant is an abstract term used to describe intermediate credentials that represent the resource owner authorization. 
An authorization grant is used by the client to obtain an access token. Several authorization grant types are defined to support a wide range of client types and user experiences. 
OAuth also allows for the definition of new extension grant types to support additional clients or to provide a bridge between OAuth and other trust frameworks. 
Finally, OAuth allows the definition of additional authentication mechanisms to be used by clients when interacting with the Authorization Server.

"Assertion Framework for OAuth 2.0 Client Authentication and Authorization Grants" [RFC7521] is an abstract extension to OAuth 2.0 that provides a general framework for the use of assertions (a.k.a. security tokens) as client credentials and/or authorization grants with OAuth 2.0. 
This specification profiles [RFC7521] to define an extension grant type that uses a self-issued Verifiable Presentation Bearer Token to request an OAuth 2.0 access token.

A Verifiable Presentation is created by a holder and can be used to prove possession of a Verifiable Credential.
The holder will need to know which Verifiable Credentials are required to access a resource.
The Authorization Server provides this information in the form of a Presentation Definition [PE].
The complete flow consists of the following steps:

1. The client requests a Presentation Definition from the Authorization Server based on an OAuth scope.
2. The client creates a Verifiable Presentation and a Presentation Submission that describes how the VP fulfills the requirements of the Presentation Definition.
3. The client requests an access token from the Authorization Server using the Verifiable Presentation as an authorization grant.
4. The Authorization Server validates the Verifiable Presentation and Presentation Submission.
5. The Authorization Server issues an access token.
6. The client uses the access token to access a resource.

## 2. Terminology

All terms are as defined in the following specifications: "The OAuth 2.0 Authorization Framework" [RFC6749], the OAuth Assertion Framework [RFC7521] "JSON Web Token (JWT)" [JWT], Verifiable Credentials Data Model 1.1 [VC-DATA-MODEL], Verifiable Credentials Presentation Exchange [PE], JSON Web Signature 2020 [JSONWebSignature2020], Decentralized Identifiers 1.0 [DID].

## 3. HTTP Parameter Bindings for Transporting Assertions

The OAuth Assertion Framework [RFC7521] defines generic HTTP parameters for transporting assertions (a.k.a. security tokens) during interactions with a token endpoint. 
This section defines specific parameters and treatments of those parameters for use with VP Bearer Tokens as Authorization Grants.

To use a VP as an authorization grant, the client uses an access token request as defined in Section 4 of the OAuth Assertion Framework [RFC7521] with the following specific parameter values and encodings.

The value of the `grant_type` is `vp_token-bearer`.

The  `assertion` parameter MUST follow the encoding requirements of the `vp_token` as defined in the OpenID for Verifiable Presentations specification [OpenID4VP].

The `scope` parameter MUST be used, as defined in the OAuth Assertion Framework [RFC6749], to indicate the requested scope. 
The scope parameter determines the set of credentials the Authorization Server expects.

The `presentation_submission` parameter MUST be used, as defined in Presentation Exchange [PE], to indicate how the VP matches the requested Verifiable Credentials.
The Presentation Submission MUST be in JSON format, URL encoded.

The following example demonstrates an access token request with a VP as an authorization grant (with extra line breaks for display purposes only):

     POST /token.oauth2 HTTP/1.1
     Host: as.example.com
     Content-Type: application/x-www-form-urlencoded

     grant_type=vp_token-bearer
     &assertion=eyJhbGciOiJFUzI1NiIsImtpZCI6IjE2In0.
     eyJpc3Mi[...omitted for brevity...].
     J9l-ZhwP[...omitted for brevity...]
     &presentation_submission=%7B...omitted for brevity...%7D
     &scope=a_scope

## 3.1 Assertion Encoding

The assertion parameter can either be encoded in JSON or JWT.
The Authorization Server determines the encoding by adding the correct parameters to the Authorization Server metadata [RFC8414]].
The `vp_formats` parameter MUST be added. It MUST contain an object defining the formats and proof types of Verifiable Presentations and Verifiable Credentials that a Verifier supports as stated by §9 of the OpenID for Verifiable Presentations specification [OpenID4VP].

## 4. JWT Format and Processing Requirements

In order to issue an access token response as described in OAuth 2.0 [RFC6749], the Authorization Server MUST validate the VP.
The validation requirements are split into three paragraphs. The first paragraph describes the requirements that apply to all VP's.
The second paragraph describes the requirements that apply to JWT encoded VP's. The third paragraph describes the requirements that apply to JSON-LD encoded VP's.

### 4.1 General processing requirements

1. The Authorization Server MUST validate the Presentation Submission according to section 6 of the Presentation Exchange [PE] specification.
2. The Authorization Server MUST validate the VP and Presentation Submission according to section 6.1 of the Presentation Exchange [PE] specification.
3. The Presentation Submission MUST fulfill the Presentation Definition that corresponds with the requested scope.
   How an Authorization Server determines the Presentation Definition based on scope is out of scope of this specification.
4. A clock skew of no more than 5 seconds MAY be applied when validating the Verifiable Presentation.

### 4.2 JWT format requirements

1. The assertion MUST be a valid JWT according to §6.3.1 of the Verifiable Credentials Data Model 1.1 [VC-DATA-MODEL].
2. The `vp` field of the JWT MUST be valid according to §4.10 the Verifiable Credentials Data Model 1.1 [VC-DATA-MODEL].
3. The `iss` field MUST be a Decentralized Identifier [DID] and match the `sub` field.
4. The `kid` header MUST be a DID URL and MUST resolve to a verificationMethod in the DID Document. The DID part of the DID URL MUST match the `iss` field.
5. The `sub` field MUST match the `credentialSubject.id` field from all the Verifiable Credentials that are used to request the access token.
6. The `aud` field MUST contain the Authorization Server URL, also known as issuer URL. [RFC8414]
7. The `nbf` field MUST be present and contain a valid unix timestamp. It MUST be before the the current time (time at which the JWT is processed).
8. The `exp` field MUST be present and contain a valid unix timestamp. It MUST be after the current time (time at which the JWT is processed).
9. The difference between the `exp` and `nbf` fields MUST be equal or less than 5 seconds.
10. The `nonce` field MUST be present and contain a string that is unique for each access token request.

### 4.3 JSON-LD format requirements

1. The assertion MUST be a valid JSON object according to §4.10 the Verifiable Credentials Data Model 1.1 [VC-DATA-MODEL].
2. The proof of the VP MUST be a valid [JSONWebSignature2020] object. Additional requirements for the proof object:
   1. The `nonce` field MUST be a string that is unique for each token request.
   2. The `domain` field MUST match the DID of the Authorization Server.
   3. The `verificationMethod` field MUST be a DID URL.
   4. The ID part of the DID in the `verificationMethod` field MUST match the `credentialSubject.id` field from all the Verifiable Credentials that are used to request the access token.
   5. The `created` field MUST be present and contain a valid ISO8601 formatted date string. It MUST be before the current time (time at which the assertion is processed).
   6. The `expires` field MUST be present and contain a valid ISO8601 formatted date string. It MUST be after the current time (time at which the assertion is processed).
   7. The difference between the `expires` and `created` fields MUST be equal or less than 5 seconds.

### 4.4 Preventing Token Replay

The Authorization Server MUST reject any request that uses a `nonce` that has been used before.
The `nonce` field in this case is determined by the client because there's no initial request to get a nonce from the Authorization Server.
The Authorization Server MUST store the unique value for 10 seconds and MUST reject any request that uses a unique value that has been used before.
The 10 seconds is based on the 5-second clock skew and the 5-second maximum difference between the expires and issued fields.

### 4.5 Error Response

If the Authorization Server determines that the VP is invalid, the Authorization Server MUST return an error response as defined in OAuth 2.0 [RFC6749].
This RFC reuses the `invalid_request` error code for all errors related to the VP.
The Authorization Server SHOULD include a description in the error response.
Errors that could occur are: 

- The signature is incorrect.
- A required field is missing.
- The Presentation Submission is not an answer to the Presentation Definition that corresponds with the requested scope.
- The submitted Verifiable Credentials do not meet the requirements of the Presentation Definition.
- The Verifiable Credentials are expired, not trusted or invalid. 
- The Verifiable Credentials are not issued to the signer of the Verifiable Presentation.

## 5. Presentation Definition endpoint

In order for a client to know which Presentation Definition [PE] to use, the Authorization Server MUST provide a Presentation Definition endpoint.
The Presentation Definition endpoint MUST be registered as a `presentation_definition_endpoint` in the Authorization Server metadata [RFC8414].
The Presentation Definition endpoint MUST return a single Presentation Definition that corresponds with the requested scope.
The client MUST support the Submission Requirement Feature [PE].
The endpoint has a single query parameter `scope` that contains the requested scope. The parameter may contain multiple values.
Values are separated by a space and MUST be URL encoded.
An empty scope MAY be used. The Authorization Server MUST return a Presentation Definition which MAY contain constraints.

The following example shows a request to the Presentation Definition endpoint:

    GET /presentation_definition?scope=a+b HTTP/1.1
    Host: as.example.com

## 6. Access Token Introspection

The Authorization Server MAY support an access token introspection endpoint as defined in OAuth 2.0 [RFC7662].
The introspection endpoint SHOULD map the fields as follows:

* `active`: If the access token is valid.
* `iss`: The issuer of the access token (DID).
* `sub`: The subject of the access token (DID), this is the resource owner. Usually the same as the `iss` field.
* `client_id`: The client (DID) that requested the access token.
* `exp`: The expiration time of the access token.
* `nbf`: The time the access token was issued.
* `scope`: The granted scope.
* `vps`: The Verifiable Presentations that were used to request the access token using the same encoding as used in the access token request.
* `presentation_submission`: The Presentation Submission that was used to request the access token.

## 7. Security Considerations

The nonce is used to prevent replay attacks. The nonce is a random string that is generated by the client.
The nonce is included in the signed data. The Authorization Server MUST reject any request that uses a nonce that has been used before.

All endpoints MUST be protected by TLS, version 1.2 as a minimum.

## 8. Privacy considerations

This RFC is meant to be used in a machine to machine context.
If any personal data may be accessed with an `vp_token-bearer` access token, it's recommended to reevaluate and use the OpenID4VP specification and include user authentication.

Extra care should be taken when designing the scope to Presentation Definition mapping.
Scopes on the personal data level should not result in different Presentation Definitions. 
This could be abused to determine if certain data is available at a Resource Server.

The Presentation Definition endpoint contains a scope parameter which could be logged in an access log.
Privacy-sensitive data in the scope parameter should be avoided.

## 9. References

* [RFC6749] D. Hardt, "The OAuth 2.0 Authorization Framework", RFC 6749, October 2012, <https://www.rfc-editor.org/info/rfc6749>.
* [RFC7521] D. Campbell, Ed., B. Zaninovich, Ed., "Assertion Framework for OAuth 2.0 Client Authentication and Authorization Grants", RFC 7521, DOI 10.17487/RFC7521, May 2015, <https://www.rfc-editor.org/info/rfc7521>.
* [RFC8414] M. Jones, N. Sakimura, J. Bradley, "OAuth 2.0 Authorization Server Metadata", RFC 8414, June 2018, <https://www.rfc-editor.org/info/rfc8414>.
* [JWT] M. Jones, J. Bradley, N. Sakimura, "JSON Web Token (JWT)", RFC 7519, May 2015, <https://www.rfc-editor.org/info/rfc7519>.
* [VC-DATA-MODEL] M. Sporny, D. Longley, D. Chadwick, "Verifiable Credentials Data Model 1.1", W3C Recommendation, 3 March 2022, <https://www.w3.org/TR/vc-data-model/>.
* [PE] D. Buchner, B. Zundel, M. Riedel, K.H. Duffy, "Presentation Exchange 2.0.0", 12 September 2023, <https://identity.foundation/presentation-exchange/spec/v2.0.0/>.
* [JSONWebSignature2020] O. Steel, M. Prorock, C.E. Lehner, "JSON Web Signature 2020", W3C Community Group Report, 21 July 2022, <https://w3c-ccg.github.io/lds-jws2020/>.
* [OpenID4VP] O. Terbu, T. Lodderstedt, K. Yasuda, T. Looker, "OpenID for Verifiable Presentations Draft 18", OpenID connect working group, 21 April 2023, <https://openid.net/specs/openid-4-verifiable-presentations-1_0.html>.
* [RFC8414] M. Jones, N. Sakimura, J. Bradley, "OAuth 2.0 Authorization Server Metadata", RFC 8414, June 2018, <https://www.rfc-editor.org/info/rfc8414>.
* [RFC7662] J. Richer, "OAuth 2.0 Token Introspection", RFC 7662, October 2015, <https://www.rfc-editor.org/info/rfc7662>.
* [DID] M. Sporny, D. Longley, M. Sabadello, D. Reed, O. Steele, C. Allen, "Decentralized Identifiers (DIDs) v1.0", W3C Recommendation, 19 July 2022, <https://www.w3.org/TR/did-core/>.