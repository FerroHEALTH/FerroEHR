import { loadRespecWithConfiguration } from "https://logius-standaarden.github.io/publicatie/respec/organisation-config.mjs";

loadRespecWithConfiguration({
  useLogo: true,
  useLabel: true,
  license: "cc-by",
  specStatus: "WV",
  specType: "HR",
  pubDomain: "logboek",
  shortName: "logboek-nen7513",
  publishDate: "2025-03-31",
  publishVersion: "0.0.1",

  latestVersion: "https://logius-standaarden.github.io/logboek-extensie-nen7513/",
  // TODO: Remove before publishing
  prevVersion: [],

  previousMaturity: "WV",
  editors:
    [
      {
        name: "Nil Barua",
        company: "Logius",
        companyURL: "https://www.logius.nl",
      },
      {
        name: "Tim van der Lippe",
        company: "Logius",
        companyURL: "https://www.logius.nl",
      }
    ],
  authors:
    [
      {
        name: "Nil Barua",
        company: "Logius",
        companyURL: "https://www.logius.nl",
      },
      {
        name: "Tim van der Lippe",
        company: "Logius",
        companyURL: "https://www.logius.nl",
      }
    ],
  github: "https://github.com/Logius-standaarden/logboek-extensie-nen7513",

  localBiblio: {
    NEN7513: {
      authors: ["Normcommissie Informatievoorziening in de zorg"],
      date: "December 2024",
      href: "https://www.nen.nl/nen-7513-2024-nl-329182",
      publisher: "NEN",
      title: "Medische informatica - Logging - Vastleggen van acties op persoonlijke gezondheidsinformatie"
  },
  }
});
